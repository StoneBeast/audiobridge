//! UDP 广播设备发现：发送端扫描局域网内的 AudioBridge 接收端。
//!
//! 线格式（小端，UDP 包，目的端口 = 服务端口，默认 48000）：
//! - 探测包：`ABRQ` (4B) + 协议版本 u8 —— 发往 255.255.255.255、
//!   各本地子网的定向广播地址以及 127.0.0.1（同机测试）；
//! - 应答包：`ABRP` (4B) + 协议版本 u8 + tcp_port u16 + name_len u16 + name UTF-8
//!   —— 接收端收到探测后向探测方**单播**应答。
//!
//! 接收端通过 [`spawn_responder`] 在 UDP/服务端口上常驻应答（与 TCP 监听同端口、
//! 不同协议栈，互不冲突）。超时/去重策略见 [`scan`]。

use std::net::{IpAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::hello::PROTOCOL_VERSION;

pub const DISCOVERY_MAGIC_PROBE: [u8; 4] = *b"ABRQ";
pub const DISCOVERY_MAGIC_REPLY: [u8; 4] = *b"ABRP";
pub const MAX_DISCOVERY_NAME: usize = 255;

/// 扫描发现的接收端设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    /// 接收端 IP（应答包源地址）。
    pub addr: IpAddr,
    /// 接收端 TCP 服务端口。
    pub tcp_port: u16,
    /// 接收端设备名。
    pub name: String,
}

/// 构造探测包（5 字节）。
pub fn probe_bytes() -> [u8; 5] {
    let mut b = [0u8; 5];
    b[..4].copy_from_slice(&DISCOVERY_MAGIC_PROBE);
    b[4] = PROTOCOL_VERSION;
    b
}

/// 构造应答包（超长名按 UTF-8 字符边界截断）。
pub fn reply_bytes(tcp_port: u16, name: &str) -> Vec<u8> {
    let name = name.as_bytes();
    let mut name_len = name.len().min(MAX_DISCOVERY_NAME);
    while name_len > 0 && std::str::from_utf8(&name[..name_len]).is_err() {
        name_len -= 1;
    }
    let mut b = Vec::with_capacity(4 + 1 + 2 + 2 + name_len);
    b.extend_from_slice(&DISCOVERY_MAGIC_REPLY);
    b.push(PROTOCOL_VERSION);
    b.extend_from_slice(&tcp_port.to_le_bytes());
    b.extend_from_slice(&(name_len as u16).to_le_bytes());
    b.extend_from_slice(&name[..name_len]);
    b
}

/// 解析应答包，返回 `(tcp_port, name)`；非法包返回 None。
pub fn parse_reply(buf: &[u8]) -> Option<(u16, String)> {
    if buf.len() < 9 || buf[..4] != DISCOVERY_MAGIC_REPLY || buf[4] != PROTOCOL_VERSION {
        return None;
    }
    let tcp_port = u16::from_le_bytes([buf[5], buf[6]]);
    let name_len = u16::from_le_bytes([buf[7], buf[8]]) as usize;
    if buf.len() < 9 + name_len {
        return None;
    }
    let name = std::str::from_utf8(&buf[9..9 + name_len]).ok()?.to_string();
    Some((tcp_port, name))
}

/// 本地子网定向广播地址（无接口信息时只发全局广播 + 回环）。
fn broadcast_targets() -> Vec<String> {
    let mut out = vec!["255.255.255.255".to_string(), "127.0.0.1".to_string()];
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for ifc in ifaces {
            if let if_addrs::IfAddr::V4(v4) = ifc.addr {
                if let Some(bc_ip) = v4.broadcast {
                    let bc = u32::from(bc_ip);
                    if bc != 0 {
                        let ip = IpAddr::V4(std::net::Ipv4Addr::from(bc)).to_string();
                        if !out.contains(&ip) {
                            out.push(ip);
                        }
                    }
                }
            }
        }
    }
    out
}

/// 发送探测并收集 `timeout_ms` 内的应答，按地址去重。
pub fn scan(discovery_port: u16, timeout_ms: u64) -> crate::Result<Vec<DiscoveredDevice>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    let probe = probe_bytes();
    for target in broadcast_targets() {
        let _ = sock.send_to(&probe, (target.as_str(), discovery_port));
    }
    sock.set_read_timeout(Some(Duration::from_millis(150)))?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut out: Vec<DiscoveredDevice> = Vec::new();
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Some((tcp_port, name)) = parse_reply(&buf[..n]) {
                    if !out.iter().any(|d| d.addr == src.ip()) {
                        out.push(DiscoveredDevice {
                            addr: src.ip(),
                            name,
                            tcp_port,
                        });
                    }
                }
            }
            Err(_) => continue, // 150ms 读超时或瞬时错误，到截止时间为止继续收
        }
    }
    out.sort_by(|a, b| a.addr.to_string().cmp(&b.addr.to_string()));
    Ok(out)
}

/// 接收端常驻应答线程：绑定 `0.0.0.0:discovery_port`（UDP），
/// 收到合法探测包即向来源单播应答。`stop` 置位后退出。
pub fn spawn_responder(
    discovery_port: u16,
    tcp_port: u16,
    device_name: String,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("ab-discover".into())
        .spawn(move || {
            let Ok(sock) = UdpSocket::bind(("0.0.0.0", discovery_port)) else {
                log::warn!("discovery responder 无法绑定 udp/{discovery_port}（可能被占用）");
                return;
            };
            let _ = sock.set_nonblocking(true);
            let reply = reply_bytes(tcp_port, &device_name);
            log::info!("discovery responder 监听 udp/{discovery_port}");
            let mut buf = [0u8; 128];
            while !stop.load(Ordering::Relaxed) {
                match sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        if n >= 5
                            && buf[..4] == DISCOVERY_MAGIC_PROBE
                            && buf[4] == PROTOCOL_VERSION
                        {
                            let _ = sock.send_to(&reply, src);
                        }
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
            log::info!("discovery responder 退出");
        })
        .expect("spawn discovery responder thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn reply_roundtrip_and_rejects() {
        let bytes = reply_bytes(48_000, "Pixel 8");
        assert_eq!(parse_reply(&bytes), Some((48_000, "Pixel 8".to_string())));
        // 错误魔数 / 错误版本 / 截断
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(parse_reply(&bad), None);
        bad = bytes.clone();
        bad[4] = 99;
        assert_eq!(parse_reply(&bad), None);
        assert_eq!(parse_reply(&bytes[..7]), None);
        // 超长名按边界截断后仍可解析
        let long = "很长的设备名 ".repeat(40);
        let bytes = reply_bytes(1, &long);
        let (_, name) = parse_reply(&bytes).unwrap();
        assert!(name.as_bytes().len() <= MAX_DISCOVERY_NAME);
        assert!(long.starts_with(&name));
    }

    #[test]
    fn probe_bytes_layout() {
        let p = probe_bytes();
        assert_eq!(&p[..4], b"ABRQ");
        assert_eq!(p[4], 1);
    }

    #[test]
    fn scan_finds_loopback_responder() {
        let stop = Arc::new(AtomicBool::new(false));
        let port = free_udp_port();
        let _resp = spawn_responder(port, port, "TEST-PC".into(), Arc::clone(&stop));
        std::thread::sleep(Duration::from_millis(200));
        let found = scan(port, 1200).unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(
            found.iter().any(|d| d.addr.is_loopback() && d.name == "TEST-PC" && d.tcp_port == port),
            "loopback responder not found: {found:?}"
        );
    }

    fn free_udp_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }
}
