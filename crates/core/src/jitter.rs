//! 接收端抖动缓冲。
//!
//! 语义：
//! - 以字节为单位的 FIFO，写入端（网络线程）[`JitterBuffer::push`]，读取端
//!   （播放线程）[`JitterBuffer::pop`]；
//! - 初始「注水」阶段：缓冲量达到目标水位前 `pop` 一律返回 0（调用方写静音）；
//! - 欠载：已启动但缓冲耗尽时，`pop` 用静音补齐并计一次 `underruns`，
//!   后续数据到达立即恢复播放（不重新注水，避免额外延迟）；
//! - 过载：缓冲超过容量上限时丢弃最旧的数据（实时流优先保新鲜度）。
//!
//! 线程模型：本身不带锁；接收侧约定为「网络线程持有 `Mutex<JitterBuffer>` 写入、
//! 播放线程周期性 `lock()` 读取」，临界区极短。

use std::collections::VecDeque;

use crate::codec::s16_bytes_per_ms;

/// 抖动缓冲运行统计。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitterStats {
    /// 累计写入字节数。
    pub pushed_bytes: u64,
    /// 累计从缓冲取出的字节数。
    pub popped_bytes: u64,
    /// 溢出时丢弃的字节数（从最旧端丢弃）。
    pub dropped_bytes: u64,
    /// 缓冲耗尽导致的静音事件次数。
    pub underruns: u64,
    /// 水位校准跳过的字节数（时钟漂移补偿，见 [`JitterBuffer::calibrate`]）。
    pub calibrated_skips: u64,
}

#[derive(Debug)]
pub struct JitterBuffer {
    queue: VecDeque<Vec<u8>>,
    chunk_bytes: usize,
    bytes_per_ms: usize,
    target_bytes: usize,
    initial_target_bytes: usize,
    capacity_bytes: usize,
    started: bool,
    /// 处于饥饿（缓冲耗尽）状态；欠载事件只在进入饥饿时计一次。
    starving: bool,
    /// 自上次静音注入以来收到的真实数据字节数（限制注入占空比）。
    real_bytes_since_inject: usize,
    stats: JitterStats,
}

impl JitterBuffer {
    /// 构造缓冲。
    ///
    /// - `frame_ms`：网络帧标称时长，仅用于统计与目标换算；
    /// - `target_ms`：启动播放前的目标水位；
    /// - `max_ms`：容量上限，超出即丢最旧。
    pub fn new(sample_rate: u32, channels: u8, frame_ms: u16, target_ms: u16, max_ms: u16) -> Self {
        let sample_rate = sample_rate.max(1);
        let channels = channels.max(1);
        let frame_ms = frame_ms.max(1);
        let bytes_per_ms = s16_bytes_per_ms(sample_rate, channels);
        let target = bytes_per_ms * target_ms.max(0) as usize;
        Self {
            queue: VecDeque::new(),
            chunk_bytes: bytes_per_ms * frame_ms as usize,
            bytes_per_ms,
            target_bytes: target,
            initial_target_bytes: target,
            capacity_bytes: (bytes_per_ms * max_ms as usize).max(bytes_per_ms * frame_ms as usize),
            started: false,
            starving: false,
            real_bytes_since_inject: 0,
            stats: JitterStats::default(),
        }
    }

    /// 写入一段音频字节（任意长度，内部按到达顺序缓存）。
    pub fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.real_bytes_since_inject = self.real_bytes_since_inject.saturating_add(data.len());
        self.stats.pushed_bytes += data.len() as u64;
        let mut overflow = (self.buffered_bytes() + data.len()).saturating_sub(self.capacity_bytes);
        while overflow > 0 {
            match self.queue.front_mut() {
                None => break,
                Some(front) => {
                    if front.len() <= overflow {
                        overflow -= front.len();
                        self.stats.dropped_bytes += front.len() as u64;
                        self.queue.pop_front();
                    } else {
                        front.drain(..overflow);
                        self.stats.dropped_bytes += overflow as u64;
                        overflow = 0;
                    }
                }
            }
        }
        self.queue.push_back(data.to_vec());
    }

    /// 读取至多 `out.len()` 字节填充 `out`。
    ///
    /// 不足的部分以静音补齐；返回值是**实际取自缓冲**的字节数
    /// （0 表示完全欠载/尚未注水完成）。`out` 总是被写满。
    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        out.fill(0);
        if out.is_empty() {
            return 0;
        }
        if !self.started {
            if self.buffered_bytes() >= self.target_bytes {
                self.started = true;
            } else {
                return 0;
            }
        }
        let was_empty = self.queue.is_empty();
        let mut filled = 0usize;
        let mut cursor = 0usize;
        while cursor < out.len() {
            match self.queue.front_mut() {
                None => break,
                Some(front) => {
                    let take = front.len().min(out.len() - cursor);
                    out[cursor..cursor + take].copy_from_slice(&front[..take]);
                    cursor += take;
                    if take == front.len() {
                        self.queue.pop_front();
                    } else {
                        front.drain(..take);
                    }
                    filled += take;
                }
            }
        }
        if was_empty && filled == 0 && !self.starving {
            self.stats.underruns += 1;
        }
        if filled == 0 {
            self.starving = true;
        } else {
            self.starving = false;
        }
        self.stats.popped_bytes += filled as u64;
        filled
    }

    /// 当前缓冲字节数。
    pub fn buffered_bytes(&self) -> usize {
        self.queue.iter().map(|v| v.len()).sum()
    }

    /// 当前缓冲时长（毫秒）。
    pub fn buffered_ms(&self) -> u64 {
        (self.buffered_bytes() / self.bytes_per_ms.max(1)) as u64
    }

    /// 单个网络帧的标称字节数。
    pub fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }

    /// 目标水位字节数。
    pub fn target_bytes(&self) -> usize {
        self.target_bytes
    }

    /// 是否已进入播放状态（注水完成）。
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// 累计统计。
    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    /// 清空缓冲（新会话开始时调用），统计保留。目标水位恢复为初始设定。
    pub fn reset(&mut self) {
        self.queue.clear();
        self.started = false;
        self.starving = false;
        self.target_bytes = self.initial_target_bytes;
    }

    /// 当前目标水位字节数。
    pub fn target_bytes_now(&self) -> usize {
        self.target_bytes
    }

    /// 水位校准：把缓冲拉回目标附近，补偿两侧时钟漂移。
    ///
    /// 长时间播放中，收发两端的采样时钟存在几十 ppm 的偏差——消费快于生产时
    /// 缓冲缓慢耗尽（周期性欠载），反之水位持续爬升（延迟越来越大）。本方法
    /// 在每次 pop 后由播放线程调用：
    /// - 缓冲 > 目标 + 2 帧：从最旧端跳过多余数据（每次最多 2 帧，听感上是
    ///   极罕见的一次轻微跳变）；
    /// - 缓冲 ≤ 目标 - 1 帧且非空：补 1 帧静音（拉长一帧，同样极罕见）；
    /// - 完全空（欠载中）不注入，等待数据恢复。
    pub fn calibrate(&mut self) -> usize {
        if !self.started {
            return 0;
        }
        let buffered = self.buffered_bytes();
        if buffered == 0 {
            return 0;
        }
        let high = self.target_bytes + 2 * self.chunk_bytes;
        if buffered > high {
            let mut drop = buffered.saturating_sub(self.target_bytes);
            drop = drop.min(2 * self.chunk_bytes);
            let mut left = drop;
            while left > 0 {
                match self.queue.front_mut() {
                    None => break,
                    Some(front) => {
                        if front.len() <= left {
                            left -= front.len();
                            self.queue.pop_front();
                        } else {
                            front.drain(..left);
                            left = 0;
                        }
                    }
                }
            }
            self.stats.calibrated_skips += drop as u64;
            drop
        } else if buffered + self.chunk_bytes <= self.target_bytes
            && self.real_bytes_since_inject >= 2 * self.chunk_bytes
        {
            // 注入占空比上限：两次注入之间至少要收到 2 帧真实数据，
            // 防止时钟严重偏差时静音充斥播放流
            let inject = self.chunk_bytes;
            self.real_bytes_since_inject = 0;
            self.queue.push_front(vec![0u8; inject]);
            inject
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> JitterBuffer {
        // 48kHz 立体声 S16：192 B/ms；20ms 帧 = 3840 B；目标 80ms，上限 200ms
        JitterBuffer::new(48_000, 2, 20, 80, 200)
    }

    #[test]
    fn primes_before_playing() {
        let mut jb = buffer();
        let mut out = [0u8; 3840];
        assert_eq!(jb.pop(&mut out), 0); // 未注水
        for _ in 0..3 {
            jb.push(&vec![1u8; 3840]);
        }
        assert!(!jb.is_started()); // 60ms < 80ms
        assert_eq!(jb.pop(&mut out), 0);
        jb.push(&vec![1u8; 3840]); // 80ms 达标
        assert_eq!(jb.pop(&mut out), 3840);
        assert!(jb.is_started());
        assert_eq!(jb.buffered_ms(), 60);
    }

    #[test]
    fn partial_pop_across_chunks() {
        let mut jb = buffer();
        for _ in 0..5 {
            jb.push(&vec![7u8; 3840]);
        }
        let mut out = [0u8; 1920]; // 半帧
        assert_eq!(jb.pop(&mut out), 1920);
        assert!(out.iter().all(|&b| b == 7));
        let mut out2 = [0u8; 3840]; // 跨块：半块旧 + 半块次新
        assert_eq!(jb.pop(&mut out2), 3840);
        assert!(out2.iter().all(|&b| b == 7));
    }

    #[test]
    fn underrun_counts_once_per_drain() {
        let mut jb = buffer();
        for _ in 0..5 {
            jb.push(&vec![1u8; 3840]);
        }
        let mut out = [0u8; 3840];
        for _ in 0..5 {
            assert_eq!(jb.pop(&mut out), 3840);
        }
        assert_eq!(jb.pop(&mut out), 0); // 第一次欠载
        assert_eq!(jb.stats().underruns, 1);
        assert_eq!(jb.pop(&mut out), 0);
        assert_eq!(jb.stats().underruns, 1); // 持续欠载只算一次
        jb.push(&vec![1u8; 3840]);
        assert_eq!(jb.pop(&mut out), 3840); // 立即恢复，不重新注水
        assert_eq!(jb.stats().underruns, 1);
    }

    #[test]
    fn overflow_drops_oldest() {
        let mut jb = buffer();
        for i in 0..11u8 {
            jb.push(&vec![i; 3840]); // 11 帧 = 42240B > 38400B 容量
        }
        assert_eq!(jb.stats().dropped_bytes, 3840);
        assert_eq!(jb.buffered_bytes(), 38400);
        let mut out = [0u8; 3840];
        jb.pop(&mut out);
        assert!(out.iter().all(|&b| b == 1)); // 最旧的 0 已被丢弃
    }

    #[test]
    fn partial_overflow_drain() {
        let mut jb = JitterBuffer::new(48_000, 2, 20, 1, 50); // 容量 9600B
        jb.push(&vec![0u8; 9600]);
        jb.push(&vec![1u8; 100]); // 溢出 100B，从最旧端截断
        assert_eq!(jb.stats().dropped_bytes, 100);
        assert_eq!(jb.buffered_bytes(), 9600);
    }

    #[test]
    fn reset_clears_but_keeps_stats() {
        let mut jb = buffer();
        for _ in 0..5 {
            jb.push(&vec![1u8; 3840]);
        }
        let mut out = [0u8; 3840];
        jb.pop(&mut out);
        let popped_before = jb.stats().popped_bytes;
        jb.reset();
        assert_eq!(jb.buffered_bytes(), 0);
        assert!(!jb.is_started());
        assert_eq!(jb.stats().popped_bytes, popped_before);
    }

    #[test]
    fn calibrate_drops_excess_back_to_target() {
        let mut jb = buffer(); // 目标 4 帧（80ms）
        for _ in 0..9 {
            jb.push(&vec![1u8; 3840]); // 9 帧
        }
        let mut out = [0u8; 3840];
        jb.pop(&mut out); // 注水完成并消费 1 帧，剩 8 帧 > 目标+2=6 帧
        let skipped = jb.calibrate();
        // 应向下修到目标(4 帧)，单次上限 2 帧 → 跳过 2 帧
        assert_eq!(skipped, 2 * 3840);
        assert_eq!(jb.buffered_bytes(), 6 * 3840);
        assert_eq!(jb.stats().calibrated_skips, 2 * 3840 as u64);
        // 剩 6 帧 == 上限，不再动作
        assert_eq!(jb.calibrate(), 0);
    }

    #[test]
    fn calibrate_injects_silence_when_behind() {
        let mut jb = buffer(); // 目标 4 帧
        let mut out = [0u8; 3840];
        // 未注水阶段 calibrate 不动作
        jb.push(&vec![1u8; 3840]);
        assert_eq!(jb.calibrate(), 0);
        jb.push(&vec![1u8; 3840]);
        jb.push(&vec![1u8; 3840]);
        jb.push(&vec![1u8; 3840]);
        jb.push(&vec![1u8; 3840]); // 5 帧，可启动
        jb.pop(&mut out); // 剩 4 帧 == 目标 → 不动
        assert_eq!(jb.calibrate(), 0);
        jb.pop(&mut out); // 剩 3 帧 ≤ 目标-1 帧 → 注入 1 帧静音
        assert_eq!(jb.calibrate(), 3840);
        assert_eq!(jb.buffered_bytes(), 4 * 3840);
        // 注入的帧在最前且是静音
        let n = jb.pop(&mut out);
        assert_eq!(n, 3840);
        assert!(out.iter().all(|&b| b == 0));
        // 占空比限制：没有新的真实数据时不允许继续注入
        jb.pop(&mut out); // 剩 2 帧
        assert_eq!(jb.calibrate(), 0);
        // 再收 2 帧真实数据后允许注入
        jb.push(&vec![1u8; 3840]);
        jb.push(&vec![1u8; 3840]); // 剩 4 帧 == 目标
        assert_eq!(jb.calibrate(), 0);
        jb.pop(&mut out); // 剩 3 帧
        assert_eq!(jb.calibrate(), 3840);
    }

    #[test]
    fn calibrate_noop_when_empty() {
        let mut jb = buffer();
        for _ in 0..4 {
            jb.push(&vec![1u8; 3840]);
        }
        let mut out = [0u8; 3840];
        for _ in 0..4 {
            jb.pop(&mut out);
        }
        // 完全耗尽（欠载）不注入，等待数据恢复
        assert_eq!(jb.calibrate(), 0);
        assert_eq!(jb.buffered_bytes(), 0);
    }

    #[test]
    fn target_accessor_and_reset() {
        let mut jb = buffer();
        assert_eq!(jb.target_bytes(), 192 * 80);
        jb.push(&vec![1u8; 3840]);
        jb.reset();
        assert_eq!(jb.target_bytes(), 192 * 80);
        assert_eq!(jb.buffered_bytes(), 0);
    }
}
