//! CLI 子进程端到端测试：tone 源 -> TCP -> null 输出。
//! 不依赖任何音频设备，可在任意 Windows 机器/CI 上运行。

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_audiobridge-cli")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn e2e_tone_over_tcp_to_null_sink() {
    let port = free_port();

    let mut listener = Command::new(bin())
        .args(["listen", "--port", &port.to_string(), "--sink", "null", "--seconds", "6"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn listener");
    // 等监听就绪
    std::thread::sleep(Duration::from_millis(800));

    let sender = Command::new(bin())
        .args([
            "send",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--tone",
            "--seconds",
            "2",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sender");
    let out = sender.wait_with_output().expect("wait sender");
    let sender_text = String::from_utf8_lossy(&out.stdout);
    assert!(sender_text.contains("SENT"), "sender summary missing: {sender_text}");

    let out = listener.wait_with_output().expect("wait listener");
    let text = String::from_utf8_lossy(&out.stdout);
    eprintln!("listener output:\n{text}");
    assert!(text.contains("RECEIVED"), "listener summary missing: {text}");

    // 解析 RECEIVED bytes=NNN
    let bytes: u64 = text
        .lines()
        .find(|l| l.starts_with("RECEIVED"))
        .and_then(|l| l.split_whitespace().find(|p| p.starts_with("bytes=")))
        .and_then(|p| p.strip_prefix("bytes="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // 2 秒 48k/2ch/s16 ≈ 384KB，下限放宽到 100KB
    assert!(bytes > 100_000, "received too little: {bytes} (text: {text})");
}
