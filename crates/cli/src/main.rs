//! AudioBridge 命令行工具。
//!
//! 用法：
//! ```text
//! audiobridge-cli send   --host <IP> [--port N] [--tone] [--seconds S] [--token T] [--name NAME]
//! audiobridge-cli listen [--port N] [--sink null|speaker] [--seconds S] [--target-ms N] [--token T] [--name NAME]
//! audiobridge-cli tone   [--freq HZ] [--seconds S]
//! ```
//! 输出统一使用 ASCII 英文，避免不同代码页下的乱码。

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiobridge_audio::{LoopbackSource, SpeakerOutput};
use audiobridge_core::session::{
    run_sender, spawn_receiver, AudioOutput, NullOutput, ReceiveOptions, ReceiverStats,
    SendOptions, ToneSource,
};
use audiobridge_core::Error;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("send") => cmd_send(&args[1..]),
        Some("listen") => cmd_listen(&args[1..]),
        Some("tone") => cmd_tone(&args[1..]),
        _ => {
            print_usage();
            2
        }
    }
}

fn print_usage() {
    eprintln!(
        "AudioBridge CLI v{}\n\n\
         USAGE:\n  \
         audiobridge-cli send   --host <IP> [--port N] [--tone] [--seconds S] [--token T] [--name NAME]\n  \
         audiobridge-cli listen [--port N] [--sink null|speaker] [--seconds S] [--target-ms N] [--token T] [--name NAME]\n  \
         audiobridge-cli tone   [--freq HZ] [--seconds S]",
        env!("CARGO_PKG_VERSION")
    );
}

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            } else {
                let key = rest.to_string();
                let value = match args.get(i + 1) {
                    Some(next) if !next.starts_with("--") => {
                        i += 1;
                        next.clone()
                    }
                    _ => "true".to_string(),
                };
                map.insert(key, value);
            }
        }
        i += 1;
    }
    map
}

fn parse_u<T: std::str::FromStr>(m: &HashMap<String, String>, key: &str, default: T) -> T {
    m.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn spawn_stop_timer(seconds: u64) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    if seconds > 0 {
        let stop2 = Arc::clone(&stop);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            stop2.store(true, Ordering::Relaxed);
        });
    }
    stop
}

fn cmd_send(args: &[String]) -> i32 {
    let m = parse_flags(args);
    let Some(host) = m.get("host").cloned() else {
        eprintln!("error: --host is required");
        return 2;
    };
    let port: u16 = parse_u(&m, "port", 48_000);
    let seconds: u64 = parse_u(&m, "seconds", 30);
    let tone = m.contains_key("tone");
    let token = m.get("token").filter(|t| !t.is_empty()).cloned();
    let name = m.get("name").cloned().unwrap_or_else(|| "Windows PC".into());
    let frame_ms: u16 = parse_u(&m, "frame-ms", 20);

    let stats = Arc::new(Mutex::new(audiobridge_core::session::SenderStats::default()));
    let stop = spawn_stop_timer(seconds);
    let opts = SendOptions {
        host: host.clone(),
        port,
        device_name: name,
        frame_ms,
        token,
    };

    println!("SEND host={host} port={port} source={} seconds={seconds}", if tone { "tone" } else { "loopback" });
    let started = Instant::now();
    let result = if tone {
        let mut src = ToneSource::new(440.0, 48_000, 2, frame_ms);
        run_sender(&opts, &mut src, Arc::clone(&stats), stop)
    } else {
        match LoopbackSource::new(frame_ms) {
            Ok(mut src) => run_sender(&opts, &mut src, Arc::clone(&stats), stop),
            Err(e) => {
                eprintln!("error: loopback capture init failed: {e}");
                return 1;
            }
        }
    };

    let s = stats.lock().unwrap().clone();
    println!(
        "SENT bytes={} dropped_chunks={} connected={} elapsed_ms={}",
        s.sent_bytes,
        s.dropped_chunks,
        s.connected,
        started.elapsed().as_millis()
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_listen(args: &[String]) -> i32 {
    let m = parse_flags(args);
    let port: u16 = parse_u(&m, "port", 48_000);
    let seconds: u64 = parse_u(&m, "seconds", 30);
    let sink = m.get("sink").cloned().unwrap_or_else(|| "speaker".into());
    let target_ms: u16 = parse_u(&m, "target-ms", 80);
    let token = m.get("token").filter(|t| !t.is_empty()).cloned();
    let name = m.get("name").cloned().unwrap_or_else(|| "Windows PC".into());
    let frame_ms: u16 = parse_u(&m, "frame-ms", 20);
    let volume: u32 = parse_u(&m, "volume", 1000);

    let stats = Arc::new(Mutex::new(ReceiverStats::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let factory: Box<dyn FnOnce() -> Result<Box<dyn AudioOutput>, Error> + Send> =
        if sink == "null" {
            Box::new(move || Ok(Box::new(NullOutput::new(frame_ms)) as Box<dyn AudioOutput>))
        } else {
            Box::new(move || {
                SpeakerOutput::new()
                    .map(|o| Box::new(o) as Box<dyn AudioOutput>)
                    .map_err(Error::Io)
            })
        };

    let _ = spawn_receiver(
        ReceiveOptions {
            port,
            device_name: name,
            frame_ms,
            target_ms,
            max_ms: 250,
            token,
            volume_permille: Arc::new(std::sync::atomic::AtomicU32::new(volume)),
        },
        Arc::clone(&stats),
        Arc::clone(&stop),
        factory,
    );
    println!("LISTEN port={port} sink={sink}");

    let deadline = Instant::now() + Duration::from_secs(seconds.max(1));
    let mut last_report = Instant::now();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            let s = stats.lock().unwrap().clone();
            println!(
                "STAT client={:?} buffered_ms={} pushed_bytes={} underruns={} dropped={}",
                s.client, s.buffered_ms, s.pushed_bytes, s.underruns, s.dropped_bytes
            );
            let _ = std::io::stdout().flush();
        }
    }
    stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));
    let s = stats.lock().unwrap().clone();
    println!(
        "RECEIVED bytes={} underruns={} dropped_bytes={}",
        s.pushed_bytes, s.underruns, s.dropped_bytes
    );
    if let Some(err) = &s.error {
        eprintln!("error: {err}");
        return 1;
    }
    0
}

fn cmd_tone(args: &[String]) -> i32 {
    let m = parse_flags(args);
    let freq: f32 = m.get("freq").and_then(|v| v.parse().ok()).unwrap_or(440.0);
    let seconds: u64 = parse_u(&m, "seconds", 3);
    let mut speaker = match SpeakerOutput::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: speaker init failed: {e}");
            return 1;
        }
    };
    println!("TONE freq={freq} seconds={seconds}");
    let mut src = ToneSource::new(freq, 48_000, 2, 20);
    let mut buf = vec![0u8; 48_000 * 2 * 2 / 1000 * 20]; // 20ms
    let deadline = Instant::now() + Duration::from_secs(seconds.max(1));
    while Instant::now() < deadline {
        match src.read(&mut buf) {
            Ok(n) => {
                if let Err(e) = speaker.write(&buf[..n]) {
                    eprintln!("error: playback failed: {e}");
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    println!("TONE done");
    0
}
