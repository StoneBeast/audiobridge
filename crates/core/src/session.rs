//! 会话层：把「音频源 -> 网络 -> 抖动缓冲 -> 音频输出」串成可运行的会话。
//!
//! 平台无关：音频采集/播放由调用方以 [`AudioSource`]/[`AudioOutput`] 接口注入
//! （桌面端用 wasapi 实现，测试/CLI 用 [`ToneSource`]/[`NullOutput`]）。
//!
//! 线程模型：
//! - 发送端（[`run_sender`]，运行在调用线程）：
//!   `AudioSource` -> `ChunkQueue` -> 网络写线程（内部启动）
//! - 接收端（[`spawn_receiver`]）：
//!   监听线程（accept）-> 连接线程 -> `Mutex<JitterBuffer>` <- 播放线程（`output_factory` 创建输出）

use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::codec::{s16_bytes_per_ms, Codec, SAMPLE_RATE};
use crate::error::Result;
use crate::framing::{
    bye_payload, read_hello, read_hello_ack, read_message, send_hello, send_hello_ack,
    write_message, DataPayload,
};
use crate::error::AckStatus;
use crate::hello::{Hello, HelloAck, MSG_BYE, MSG_DATA, MSG_PING, MSG_PONG};
use crate::jitter::JitterBuffer;
use crate::pcm::apply_volume_s16;
use crate::queue::{ChunkQueue, Recv};

/// 音频源：阻塞式读取，返回填入字节数。
///
/// 约定：
/// - 返回 `Ok(0)` 表示「本时刻没有数据，请继续」（不是结束）；
/// - 源内部必须自带实时节拍（不能无限制快于实时），例如 loopback 由事件驱动、
///   [`ToneSource`] 内部按帧时长休眠；
/// - 会话结束由 `stop` 标志控制，而不是让 read 返回错误。
///
/// 注意：不要求 `Send`——WASAPI 等 COM 对象不可跨线程，
/// 实现应始终在创建它的同一线程上使用（见桌面端用法）。
pub trait AudioSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

/// 音频输出：阻塞式写入（按实时速率消费），实现方出错时返回 Err。
/// 同样不要求 `Send`（在播放线程内部创建并使用）。
pub trait AudioOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<()>;
}

/// 测试/演示用：生成正弦波音频源（按帧时长节拍）。
pub struct ToneSource {
    freq_hz: f32,
    amplitude: f32,
    rate: u32,
    channels: u8,
    chunk_bytes: usize,
    phase: u64,
}

impl ToneSource {
    pub fn new(freq_hz: f32, rate: u32, channels: u8, frame_ms: u16) -> Self {
        Self {
            freq_hz,
            amplitude: 0.25,
            rate,
            channels,
            chunk_bytes: s16_bytes_per_ms(rate, channels) * frame_ms.max(1) as usize,
            phase: 0,
        }
    }
}

impl AudioSource for ToneSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let ch = self.channels.max(1) as usize;
        let frames = self.chunk_bytes / (ch * 2);
        let mut samples = Vec::with_capacity(frames * ch);
        for i in 0..frames {
            let t = ((self.phase + i as u64) as f64 / self.rate as f64) as f32;
            let v = (2.0 * std::f32::consts::PI * self.freq_hz * t).sin() * self.amplitude;
            for _ in 0..ch {
                samples.push(v);
            }
        }
        self.phase += frames as u64;
        let n = crate::pcm::f32_to_s16_bytes(&samples, buf);
        std::thread::sleep(Duration::from_millis(
            (frames as u64 * 1000) / self.rate.max(1) as u64,
        ));
        Ok(n)
    }
}

/// 测试用：以帧时长节拍消费的空输出。
pub struct NullOutput {
    chunk_ms: u64,
}

impl NullOutput {
    pub fn new(frame_ms: u16) -> Self {
        Self {
            chunk_ms: frame_ms.max(1) as u64,
        }
    }
}

impl AudioOutput for NullOutput {
    fn write(&mut self, _buf: &[u8]) -> io::Result<()> {
        std::thread::sleep(Duration::from_millis(self.chunk_ms));
        Ok(())
    }
}

/// 访问令牌 -> SHA-256（发送端放入 HELLO，接收端比对）。
pub fn token_hash(token: &str) -> [u8; 32] {
    if token.is_empty() {
        return [0u8; 32];
    }
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

#[derive(Debug, Default, Clone)]
pub struct SenderStats {
    pub connected: bool,
    pub peer: String,
    pub sent_bytes: u64,
    pub dropped_chunks: u64,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ReceiverStats {
    pub client: Option<String>,
    pub buffered_ms: u64,
    pub pushed_bytes: u64,
    pub underruns: u64,
    pub dropped_bytes: u64,
    pub error: Option<String>,
}

fn upd<T>(m: &Mutex<T>, f: impl FnOnce(&mut T)) {
    if let Ok(mut g) = m.lock() {
        f(&mut g);
    }
}

#[derive(Debug, Clone)]
pub struct SendOptions {
    pub host: String,
    pub port: u16,
    pub device_name: String,
    pub frame_ms: u16,
    pub token: Option<String>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 48000,
            device_name: "PC".into(),
            frame_ms: crate::codec::DEFAULT_FRAME_MS,
            token: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReceiveOptions {
    pub port: u16,
    pub device_name: String,
    pub frame_ms: u16,
    pub target_ms: u16,
    pub max_ms: u16,
    pub token: Option<String>,
    /// 播放音量（千分比），可运行时调整。
    pub volume_permille: Arc<AtomicU32>,
}

impl Default for ReceiveOptions {
    fn default() -> Self {
        Self {
            port: 48000,
            device_name: "PC".into(),
            frame_ms: crate::codec::DEFAULT_FRAME_MS,
            target_ms: crate::codec::DEFAULT_TARGET_MS,
            max_ms: 250,
            token: None,
            volume_permille: Arc::new(AtomicU32::new(1000)),
        }
    }
}

/// 运行发送会话（阻塞，直到 `stop` 置位或出错）。
///
/// 调用线程充当采集线程：连接+握手成功后启动网络写线程，然后循环
/// `source.read()` -> 队列。返回时已尽力发送 BYE 并回收写线程。
pub fn run_sender(
    opts: &SendOptions,
    source: &mut dyn AudioSource,
    stats: Arc<Mutex<SenderStats>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let chunk_bytes = s16_bytes_per_ms(SAMPLE_RATE, crate::codec::CHANNELS) * opts.frame_ms.max(1) as usize;
    let hello = Hello {
        device_name: opts.device_name.clone(),
        codec: Codec::PcmS16Le,
        sample_rate: SAMPLE_RATE,
        channels: crate::codec::CHANNELS,
        frame_ms: opts.frame_ms,
        buffer_target_ms: crate::codec::DEFAULT_TARGET_MS,
        auth_token_sha256: token_hash(opts.token.as_deref().unwrap_or("")),
    };

    log::info!("connecting to {}:{}", opts.host, opts.port);
    let stream = TcpStream::connect((opts.host.as_str(), opts.port))?;
    stream.set_nodelay(true)?;
    let mut write_half = stream.try_clone()?;

    send_hello(&mut write_half, &hello)?;
    let ack = read_hello_ack(&mut write_half)?;
    log::info!("handshake ok: {ack}");

    upd(&stats, |s| {
        s.connected = true;
        s.peer = format!("{}:{}（{}）", opts.host, opts.port, ack.device_name);
        s.error = None;
        s.sent_bytes = 0;
    });

    let queue = Arc::new(ChunkQueue::<Vec<u8>>::new(150));
    let net_stop = Arc::new(AtomicBool::new(false));
    let net_stats = Arc::clone(&stats);
    let net_queue = Arc::clone(&queue);
    let net_stop_flag = Arc::clone(&net_stop);
    let net_thread = std::thread::Builder::new()
        .name("ab-net-send".into())
        .spawn(move || {
            let mut seq: u32 = 0;
            let started = Instant::now();
            while !net_stop_flag.load(Ordering::Relaxed) {
                match net_queue.pop_timeout(Duration::from_millis(250)) {
                    Recv::Item(chunk) => {
                        let payload = DataPayload {
                            seq: seq,
                            timestamp_ms: started.elapsed().as_millis() as u32,
                            audio: chunk,
                        };
                        if let Err(e) = write_message(&mut write_half, MSG_DATA, &payload.to_bytes()) {
                            log::warn!("send failed: {e}");
                            upd(&net_stats, |s| s.error = Some(format!("发送失败: {e}")));
                            net_stop_flag.store(true, Ordering::Relaxed);
                            let _ = write_half.shutdown(Shutdown::Both);
                            break;
                        }
                        seq = seq.wrapping_add(1);
                        upd(&net_stats, |s| s.sent_bytes += payload.audio.len() as u64);
                    }
                    Recv::TimedOut => {}
                    Recv::Closed => break,
                }
            }
            // 礼节性 BYE（对端可能已关闭，忽略错误）
            let _ = write_message(&mut write_half, MSG_BYE, &bye_payload(0, "bye"));
            let _ = write_half.shutdown(Shutdown::Both);
        })?;

    let mut buf = vec![0u8; chunk_bytes];
    let mut result = Ok(());
    while !stop.load(Ordering::Relaxed) && !net_stop.load(Ordering::Relaxed) {
        match source.read(&mut buf) {
            Ok(0) => continue, // 本时刻无数据（静音等）
            Ok(n) => {
                let dropped = queue.push(buf[..n].to_vec());
                if dropped > 0 {
                    upd(&stats, |s| s.dropped_chunks = dropped);
                }
            }
            Err(e) => {
                log::warn!("capture source error: {e}");
                upd(&stats, |s| s.error = Some(format!("采集失败: {e}")));
                result = Err(crate::error::Error::Io(e));
                break;
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    net_stop.store(true, Ordering::Relaxed);
    queue.close();
    let _ = stream.shutdown(Shutdown::Both);
    let _ = net_thread.join();
    upd(&stats, |s| s.connected = false);
    result
}

/// 启动接收会话（监听线程 + 连接线程 + 播放线程），立即返回线程句柄。
pub fn spawn_receiver<F>(
    opts: ReceiveOptions,
    stats: Arc<Mutex<ReceiverStats>>,
    stop: Arc<AtomicBool>,
    output_factory: F,
) -> Vec<JoinHandle<()>>
where
    F: FnOnce() -> Result<Box<dyn AudioOutput>> + Send + 'static,
{
    let jitter = Arc::new(Mutex::new(JitterBuffer::new(
        SAMPLE_RATE,
        crate::codec::CHANNELS,
        opts.frame_ms,
        opts.target_ms,
        opts.max_ms,
    )));
    let active = Arc::new(AtomicBool::new(false));

    // 播放线程
    {
        let jitter = Arc::clone(&jitter);
        let stats = Arc::clone(&stats);
        let stop = Arc::clone(&stop);
        let volume = Arc::clone(&opts.volume_permille);
        let chunk_bytes = s16_bytes_per_ms(SAMPLE_RATE, crate::codec::CHANNELS)
            * opts.frame_ms.max(1) as usize;
        let t = std::thread::Builder::new()
            .name("ab-playback".into())
            .spawn(move || {
                let mut out = match output_factory() {
                    Ok(o) => o,
                    Err(e) => {
                        log::error!("audio output init failed: {e}");
                        upd(&stats, |s| s.error = Some(format!("播放设备初始化失败: {e}")));
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                log::info!("playback thread started");
                let mut buf = vec![0u8; chunk_bytes];
                while !stop.load(Ordering::Relaxed) {
                    {
                        let mut j = jitter.lock().unwrap();
                        j.pop(&mut buf);
                        let st = j.stats();
                        upd(&stats, |s| {
                            s.buffered_ms = j.buffered_ms();
                            s.underruns = st.underruns;
                            s.dropped_bytes = st.dropped_bytes;
                            s.pushed_bytes = st.pushed_bytes;
                        });
                    }
                    apply_volume_s16(&mut buf, volume.load(Ordering::Relaxed));
                    if let Err(e) = out.write(&buf) {
                        log::warn!("playback write failed: {e}");
                        upd(&stats, |s| s.error = Some(format!("播放失败: {e}")));
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                log::info!("playback thread exiting");
            })
            .expect("spawn playback thread");
        let _ = t;
    }

    // 监听线程（nonblocking accept 轮询，保证可被 stop 唤醒）
    {
        let jitter = Arc::clone(&jitter);
        let stats = Arc::clone(&stats);
        let stop = Arc::clone(&stop);
        let active = Arc::clone(&active);
        let opts = opts.clone();
        let t = std::thread::Builder::new()
            .name("ab-listen".into())
            .spawn(move || {
                let listener = match TcpListener::bind(("0.0.0.0", opts.port)) {
                    Ok(l) => l,
                    Err(e) => {
                        upd(&stats, |s| s.error = Some(format!("监听 {0} 端口失败: {e}", opts.port)));
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                let _ = listener.set_nonblocking(true);
                log::info!("listening on 0.0.0.0:{}", opts.port);
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, addr)) => {
                            let jitter = Arc::clone(&jitter);
                            let stats = Arc::clone(&stats);
                            let stop = Arc::clone(&stop);
                            let active = Arc::clone(&active);
                            let opts = opts.clone();
                            let t = std::thread::Builder::new()
                                .name("ab-conn".into())
                                .spawn(move || {
                                    conn_loop(stream, addr.to_string(), &opts, jitter, stats, stop, active);
                                });
                            if t.is_err() {
                                log::error!("spawn conn thread failed");
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(e) => {
                            upd(&stats, |s| s.error = Some(format!("accept 失败: {e}")));
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                }
                log::info!("listener exiting");
            })
            .expect("spawn listener thread");
        let _ = t;
    }

    Vec::new() // 线程自主运行，通过 stop 标志终止（句柄由会话自然回收）
}

#[allow(clippy::too_many_arguments)]
fn conn_loop(
    mut stream: TcpStream,
    addr: String,
    opts: &ReceiveOptions,
    jitter: Arc<Mutex<JitterBuffer>>,
    stats: Arc<Mutex<ReceiverStats>>,
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
) {
    // Windows 上 accept 出的 socket 会继承监听端的非阻塞标志，必须恢复阻塞模式
    stream.set_nonblocking(false).ok();
    stream.set_nodelay(true).ok();

    // 已有活跃会话 -> BUSY
    if active.load(Ordering::Relaxed) {
        let ack = HelloAck {
            status: AckStatus::Busy,
            codec: crate::codec::Codec::PcmS16Le,
            sample_rate: SAMPLE_RATE,
            channels: crate::codec::CHANNELS,
            device_name: opts.device_name.clone(),
            jitter_target_ms: opts.target_ms,
        };
        let _ = send_hello_ack(&mut stream, &ack);
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    let hello = match read_hello(&mut stream) {
        Ok(h) => h,
        Err(e) => {
            log::info!("bad hello from {addr}: {e}");
            return;
        }
    };

    // 鉴权：接收端设置了令牌时，发送端必须出示匹配摘要
    let token_ok = match &opts.token {
        None => true,
        Some(t) => hello.auth_token_sha256 == token_hash(t),
    };
    let status = if !token_ok {
        AckStatus::AuthFailed
    } else {
        AckStatus::Ok
    };
    let ack = HelloAck {
        status,
        codec: crate::codec::Codec::PcmS16Le,
        sample_rate: SAMPLE_RATE,
        channels: crate::codec::CHANNELS,
        device_name: opts.device_name.clone(),
        jitter_target_ms: opts.target_ms,
    };
    if send_hello_ack(&mut stream, &ack).is_err() {
        return;
    }
    if status != AckStatus::Ok {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    active.store(true, Ordering::Relaxed);
    jitter.lock().unwrap().reset();
    upd(&stats, |s| {
        s.client = Some(format!("{addr}（{}）", hello.device_name));
        s.error = None;
    });
    log::info!("client connected: {hello}");

    let mut payload = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match read_message(&mut stream, &mut payload) {
            Ok(MSG_DATA) => {
                match DataPayload::from_bytes(&payload) {
                    Ok(dp) => jitter.lock().unwrap().push(&dp.audio),
                    Err(e) => {
                        log::warn!("bad data payload: {e}");
                        break;
                    }
                };
            }
            Ok(MSG_PING) => {
                let _ = write_message(&mut stream, MSG_PONG, &payload);
            }
            Ok(MSG_PONG) => {}
            Ok(MSG_BYE) => {
                log::info!("peer sent BYE");
                break;
            }
            Ok(other) => log::debug!("ignoring msg type {other}"),
            Err(e) => {
                log::info!("connection closed ({e})");
                break;
            }
        }
    }

    active.store(false, Ordering::Relaxed);
    jitter.lock().unwrap().reset();
    upd(&stats, |s| s.client = None);
    let _ = write_message(&mut stream, MSG_BYE, &bye_payload(0, "session end"));
    let _ = stream.shutdown(Shutdown::Both);
}
