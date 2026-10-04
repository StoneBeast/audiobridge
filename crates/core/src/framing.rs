//! 流式消息帧与握手的读写。
//!
//! 握手完成后，双向通道上的每条消息格式为：
//! ```text
//! [msg_type u8][payload_len u16][payload ...]
//! ```
//! DATA 帧的 payload：`[seq u32][timestamp_ms u32][audio bytes]`。

use std::io::{Read, Write};

use crate::codec::MAX_PAYLOAD;
use crate::error::{Error, Result};
use crate::hello::{Hello, HelloAck, MAGIC, MSG_BYE, MSG_DATA, MSG_HELLO, MSG_HELLO_ACK, MSG_PING, MSG_PONG, PROTOCOL_VERSION};

/// 写出握手消息（HELLO 或 HELLO_ACK）。
fn write_handshake(w: &mut impl Write, msg_type: u8, body: &[u8]) -> Result<()> {
    if body.len() > MAX_PAYLOAD {
        return Err(Error::PayloadTooLarge(body.len()));
    }
    w.write_all(&MAGIC)?;
    w.write_all(&[PROTOCOL_VERSION, msg_type])?;
    w.write_all(&(body.len() as u16).to_le_bytes())?;
    w.write_all(body)?;
    Ok(())
}

/// 读取握手消息头，校验魔数/版本/类型，返回消息体长度。
fn read_handshake_head(r: &mut impl Read, expected_type: u8) -> Result<usize> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head)?;
    if head[0..4] != MAGIC {
        return Err(Error::BadMagic);
    }
    if head[4] != PROTOCOL_VERSION {
        return Err(Error::UnsupportedVersion(head[4]));
    }
    if head[5] != expected_type {
        return Err(Error::UnexpectedType(head[5]));
    }
    Ok(u16::from_le_bytes([head[6], head[7]]) as usize)
}

fn read_handshake_body(r: &mut impl Read, expected_type: u8) -> Result<Vec<u8>> {
    let len = read_handshake_head(r, expected_type)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(body)
}

/// 发送端 -> 接收端：发送 HELLO。
pub fn send_hello(w: &mut impl Write, hello: &Hello) -> Result<()> {
    write_handshake(w, MSG_HELLO, &hello.to_bytes())
}

/// 接收端：读取 HELLO。
pub fn read_hello(r: &mut impl Read) -> Result<Hello> {
    let body = read_handshake_body(r, MSG_HELLO)?;
    Hello::from_bytes(&body)
}

/// 接收端 -> 发送端：发送 HELLO_ACK。
pub fn send_hello_ack(w: &mut impl Write, ack: &HelloAck) -> Result<()> {
    write_handshake(w, MSG_HELLO_ACK, &ack.to_bytes())
}

/// 发送端：读取 HELLO_ACK。
pub fn read_hello_ack(r: &mut impl Read) -> Result<HelloAck> {
    let body = read_handshake_body(r, MSG_HELLO_ACK)?;
    let ack = HelloAck::from_bytes(&body)?;
    if ack.status != crate::error::AckStatus::Ok {
        return Err(Error::Rejected(ack.status));
    }
    Ok(ack)
}

/// 发送端：读取 HELLO_ACK 但不因拒绝状态报错（调用方自行检查 `ack.status`）。
pub fn read_hello_ack_raw(r: &mut impl Read) -> Result<HelloAck> {
    let body = read_handshake_body(r, MSG_HELLO_ACK)?;
    HelloAck::from_bytes(&body)
}

/// 写出一条握手后的流式消息帧。
pub fn write_message(w: &mut impl Write, msg_type: u8, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_PAYLOAD {
        return Err(Error::PayloadTooLarge(payload.len()));
    }
    w.write_all(&[msg_type])?;
    w.write_all(&(payload.len() as u16).to_le_bytes())?;
    w.write_all(payload)?;
    Ok(())
}

/// 读入一条握手后的流式消息帧。清空并复用 `payload` 缓冲，返回消息类型。
pub fn read_message(r: &mut impl Read, payload: &mut Vec<u8>) -> Result<u8> {
    let mut hdr = [0u8; 3];
    r.read_exact(&mut hdr)?;
    let msg_type = hdr[0];
    let len = u16::from_le_bytes([hdr[1], hdr[2]]) as usize;
    payload.clear();
    payload.reserve(len);
    payload.resize(len, 0);
    r.read_exact(payload)?;
    Ok(msg_type)
}

/// DATA 帧的负载。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPayload {
    /// 发送端维护的单调递增序号（回绕安全：仅用于丢包统计）。
    pub seq: u32,
    /// 发送端单调时钟毫秒数（mod 2^32），仅供统计参考，不参与缓冲决策。
    pub timestamp_ms: u32,
    /// 音频负载（编码由握手协商，v1 为交错 S16LE）。
    pub audio: Vec<u8>,
}

impl DataPayload {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(8 + self.audio.len());
        b.extend_from_slice(&self.seq.to_le_bytes());
        b.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        b.extend_from_slice(&self.audio);
        b
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 {
            return Err(Error::Truncated { need: 8, got: buf.len() });
        }
        let seq = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let timestamp_ms = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        Ok(Self {
            seq,
            timestamp_ms,
            audio: buf[8..].to_vec(),
        })
    }
}

/// BYE 帧的负载：`[reason u8][可选 UTF-8 说明]`。
pub fn bye_payload(reason: u8, note: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(1 + note.len());
    b.push(reason);
    b.extend_from_slice(note.as_bytes());
    b
}

/// 解析 BYE 帧负载，返回 `(reason, note)`。
pub fn parse_bye_payload(payload: &[u8]) -> (u8, String) {
    match payload.first() {
        None => (0, String::new()),
        Some(&reason) => {
            let note = std::str::from_utf8(&payload[1..]).unwrap_or("").to_string();
            (reason, note)
        }
    }
}

/// 便捷封装：PING/PONG 负载（4 字节随机数）。
pub fn ping_payload(nonce: u32) -> [u8; 4] {
    nonce.to_le_bytes()
}

pub fn parse_ping_payload(payload: &[u8]) -> Option<u32> {
    if payload.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;
    use std::io::Cursor;

    #[test]
    fn hello_exchange_over_cursor() {
        let hello = Hello {
            device_name: "Pixel 8".into(),
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            buffer_target_ms: 80,
            auth_token_sha256: [0u8; 32],
        };
        let ack = HelloAck {
            status: crate::error::AckStatus::Ok,
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            device_name: "DESKTOP-PC".into(),
            jitter_target_ms: 80,
        };

        let mut wire = Vec::new();
        send_hello(&mut wire, &hello).unwrap();
        send_hello_ack(&mut wire, &ack).unwrap();

        let mut cur = Cursor::new(&wire);
        let got_hello = read_hello(&mut cur).unwrap();
        assert_eq!(got_hello, hello);
        let got_ack = read_hello_ack(&mut cur).unwrap();
        assert_eq!(got_ack, ack);
    }

    #[test]
    fn rejected_ack_surfaces_as_error() {
        let ack = HelloAck {
            status: crate::error::AckStatus::Busy,
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            device_name: "PC".into(),
            jitter_target_ms: 80,
        };
        let mut wire = Vec::new();
        send_hello_ack(&mut wire, &ack).unwrap();
        let mut cur = Cursor::new(&wire);
        match read_hello_ack(&mut cur) {
            Err(Error::Rejected(crate::error::AckStatus::Busy)) => {}
            other => panic!("expected Rejected(Busy), got {other:?}"),
        }
    }

    #[test]
    fn message_roundtrip_and_stream() {
        let payload = DataPayload {
            seq: 1,
            timestamp_ms: 999,
            audio: vec![0u8; 3840],
        };
        let mut wire = Vec::new();
        write_message(&mut wire, MSG_DATA, &payload.to_bytes()).unwrap();
        write_message(&mut wire, MSG_PING, &ping_payload(0xDEADBEEF)).unwrap();
        write_message(&mut wire, MSG_BYE, &bye_payload(0, "bye")).unwrap();

        let mut cur = Cursor::new(&wire);
        let mut buf = Vec::new();
        let t = read_message(&mut cur, &mut buf).unwrap();
        assert_eq!(t, MSG_DATA);
        let got = DataPayload::from_bytes(&buf).unwrap();
        assert_eq!(got, payload);

        let t = read_message(&mut cur, &mut buf).unwrap();
        assert_eq!(t, MSG_PING);
        assert_eq!(parse_ping_payload(&buf), Some(0xDEADBEEF));

        let t = read_message(&mut cur, &mut buf).unwrap();
        assert_eq!(t, MSG_BYE);
        assert_eq!(parse_bye_payload(&buf), (0, "bye".to_string()));
    }

    #[test]
    fn data_payload_min_size() {
        let p = DataPayload::from_bytes(&[1, 0, 0, 0, 2, 0, 0, 0]).unwrap();
        assert_eq!(p.seq, 1);
        assert_eq!(p.timestamp_ms, 2);
        assert!(p.audio.is_empty());
    }

    #[test]
    fn bad_magic_detected() {
        let mut wire = Vec::new();
        send_hello(&mut wire, &Hello {
            device_name: "x".into(),
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            buffer_target_ms: 80,
            auth_token_sha256: [0; 32],
        })
        .unwrap();
        wire[0] = b'X';
        let mut cur = Cursor::new(&wire);
        assert!(matches!(read_hello(&mut cur), Err(Error::BadMagic)));
    }
}
