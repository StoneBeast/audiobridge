//! 系统声音 loopback 采集源（WASAPI 渲染设备 + LOOPBACK 标志）。

use std::collections::VecDeque;
use std::io;

use audiobridge_core::session::AudioSource;
use wasapi::{initialize_mta, AudioClient, Direction, DeviceEnumerator, Handle, SampleType, StreamMode, WaveFormat};

/// 采集系统正在播放的音频（loopback）。
///
/// 内部以「攒够 `frame_ms` 毫秒数据再返回」的策略平滑 WASAPI 事件节拍；
/// 系统静音时事件不触发，超时后返回 `Ok(0)`（会话层据此保活）。
pub struct LoopbackSource {
    audio_client: AudioClient,
    capture_client: wasapi::AudioCaptureClient,
    event: Handle,
    deque: VecDeque<u8>,
    pending: Vec<u8>,
    target_bytes: usize,
}

impl LoopbackSource {
    pub fn new(frame_ms: u16) -> Result<Self, wasapi::WasapiError> {
        initialize_mta().ok()?;
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut audio_client = device.get_iaudioclient()?;

        // 请求 48k/2ch/S16；autoconvert 由引擎完成任意格式转换
        let format = WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None);
        let (def_time, _min_time) = audio_client.get_device_period()?;
        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: def_time * 8,
        };
        // 在渲染设备上以 Capture 方向初始化 -> 自动加 LOOPBACK 标志
        audio_client.initialize_client(&format, &Direction::Capture, &mode)?;
        let event = audio_client.set_get_eventhandle()?;
        let capture_client = audio_client.get_audiocaptureclient()?;
        audio_client.start_stream()?;

        let target_bytes = wasapi_ok_bytes(0, frame_ms);
        log::info!("loopback capture started (target chunk = {target_bytes} B)");
        Ok(Self {
            audio_client,
            capture_client,
            event,
            deque: VecDeque::with_capacity(64 * 1024),
            pending: Vec::with_capacity(target_bytes * 2),
            target_bytes,
        })
    }
}

fn wasapi_ok_bytes(_blockalign: usize, frame_ms: u16) -> usize {
    // 48_000 Hz * 2 声道 * 2 字节 / 1000 ms
    48_000 * 2 * 2 / 1000 * frame_ms.max(1) as usize
}

impl Drop for LoopbackSource {
    fn drop(&mut self) {
        let _ = self.audio_client.stop_stream();
        log::info!("loopback capture stopped");
    }
}

impl AudioSource for LoopbackSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let target = self.target_bytes.min(buf.len());
        loop {
            // 已攒够 -> 返回
            if self.pending.len() >= target {
                buf[..target].copy_from_slice(&self.pending[..target]);
                self.pending.drain(..target);
                return Ok(target);
            }

            // 读取设备缓冲中已积累的包
            match self.capture_client.read_from_device_to_deque(&mut self.deque) {
                Ok(_info) => {
                    self.pending.extend(self.deque.drain(..));
                }
                Err(e) => return Err(io::Error::other(e)),
            }

            if self.pending.len() >= target {
                continue; // 再走一圈顶部的「攒够」分支
            }

            // 等下一个事件；超时（系统静音时 loopback 不触发事件）
            match self.event.wait_for_event(200) {
                Ok(()) => continue,
                Err(_) => {
                    // 有零头也先发出去，保持接收端流活跃
                    let n = self.pending.len().min(buf.len());
                    if n > 0 {
                        buf[..n].copy_from_slice(&self.pending[..n]);
                        self.pending.drain(..n);
                        return Ok(n);
                    }
                    return Ok(0);
                }
            }
        }
    }
}
