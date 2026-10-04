//! 握手消息（HELLO / HELLO_ACK）的编解码。
//!
//! 线格式（小端）：
//! ```text
//! 偏移  长度  字段
//! 0     4     魔数 "ABRG"
//! 4     1     协议版本（当前 1）
//! 5     1     消息类型（1 = HELLO，2 = HELLO_ACK）
//! 6     2     消息体长度 N（u16）
//! 8     N     消息体
//! ```
//! HELLO 消息体：`[name_len u16][name][codec u8][sample_rate u32][channels u8]
//! [frame_ms u16][buffer_target_ms u16][token_sha256 32B]`
//!
//! HELLO_ACK 消息体：`[status u8][codec u8][sample_rate u32][channels u8]
//! [name_len u16][name][jitter_target_ms u16]`

use crate::codec::{Codec, MAX_NAME_LEN};
use crate::error::{Error, Result, AckStatus};

/// 线协议魔数。
pub const MAGIC: [u8; 4] = *b"ABRG";
/// 当前协议版本。
pub const PROTOCOL_VERSION: u8 = 1;
/// 客户端 -> 服务端：请求推流。
pub const MSG_HELLO: u8 = 1;
/// 服务端 -> 客户端：握手应答。
pub const MSG_HELLO_ACK: u8 = 2;
/// 推流方向：音频数据帧。
pub const MSG_DATA: u8 = 3;
/// 双向：心跳请求。
pub const MSG_PING: u8 = 4;
/// 双向：心跳应答。
pub const MSG_PONG: u8 = 5;
/// 双向：结束会话。
pub const MSG_BYE: u8 = 6;

/// 发起方（音频发送端）在握手阶段发出的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// 发送端设备名（用于 UI 展示，UTF-8，最长 255 字节）。
    pub device_name: String,
    /// 请求的负载编码。
    pub codec: Codec,
    /// 线协议采样率。
    pub sample_rate: u32,
    /// 线协议声道数。
    pub channels: u8,
    /// 每个 DATA 帧承载的标称时长（毫秒）。
    pub frame_ms: u16,
    /// 发送端建议的接收端抖动缓冲水位（毫秒）。
    pub buffer_target_ms: u16,
    /// 可选的访问令牌 SHA-256 摘要；全零表示不校验。
    pub auth_token_sha256: [u8; 32],
}

impl Hello {
    pub fn to_bytes(&self) -> Vec<u8> {
        let name = self.device_name.as_bytes();
        let name_len = name.len().min(MAX_NAME_LEN);
        let mut b = Vec::with_capacity(2 + name_len + 1 + 4 + 1 + 2 + 2 + 32);
        b.extend_from_slice(&(name_len as u16).to_le_bytes());
        b.extend_from_slice(&name[..name_len]);
        b.push(self.codec.to_u8());
        b.extend_from_slice(&self.sample_rate.to_le_bytes());
        b.push(self.channels);
        b.extend_from_slice(&self.frame_ms.to_le_bytes());
        b.extend_from_slice(&self.buffer_target_ms.to_le_bytes());
        b.extend_from_slice(&self.auth_token_sha256);
        b
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut r = Reader::new(buf);
        let name_len = r.u16()? as usize;
        if name_len > MAX_NAME_LEN {
            return Err(Error::PayloadTooLarge(name_len));
        }
        let device_name = std::str::from_utf8(r.take(name_len)?)?.to_string();
        let codec_val = r.u8()?;
        let codec = Codec::from_u8(codec_val).ok_or(Error::UnknownCodec(codec_val))?;
        let sample_rate = r.u32()?;
        let channels = r.u8()?;
        let frame_ms = r.u16()?;
        let buffer_target_ms = r.u16()?;
        let mut auth_token_sha256 = [0u8; 32];
        auth_token_sha256.copy_from_slice(r.take(32)?);
        if sample_rate == 0 || frame_ms == 0 {
            return Err(Error::InvalidField("sample_rate/frame_ms"));
        }
        if channels == 0 || channels > 8 {
            return Err(Error::InvalidField("channels"));
        }
        Ok(Self {
            device_name,
            codec,
            sample_rate,
            channels,
            frame_ms,
            buffer_target_ms,
            auth_token_sha256,
        })
    }
}

/// 接收方（音频接收端）对 HELLO 的应答。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloAck {
    /// 接受或拒绝（拒绝时连接应关闭）。
    pub status: AckStatus,
    /// 协商后的负载编码。
    pub codec: Codec,
    /// 协商后的采样率。
    pub sample_rate: u32,
    /// 协商后的声道数。
    pub channels: u8,
    /// 接收端设备名。
    pub device_name: String,
    /// 接收端实际使用的抖动缓冲水位（毫秒）。
    pub jitter_target_ms: u16,
}

impl HelloAck {
    pub fn to_bytes(&self) -> Vec<u8> {
        let name = self.device_name.as_bytes();
        let name_len = name.len().min(MAX_NAME_LEN);
        let mut b = Vec::with_capacity(1 + 1 + 4 + 1 + 2 + name_len + 2);
        b.push(self.status.to_u8());
        b.push(self.codec.to_u8());
        b.extend_from_slice(&self.sample_rate.to_le_bytes());
        b.push(self.channels);
        b.extend_from_slice(&(name_len as u16).to_le_bytes());
        b.extend_from_slice(&name[..name_len]);
        b.extend_from_slice(&self.jitter_target_ms.to_le_bytes());
        b
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        let mut r = Reader::new(buf);
        let status = AckStatus::from_u8(r.u8()?);
        let codec_val = r.u8()?;
        let codec = Codec::from_u8(codec_val).ok_or(Error::UnknownCodec(codec_val))?;
        let sample_rate = r.u32()?;
        let channels = r.u8()?;
        let name_len = r.u16()? as usize;
        if name_len > MAX_NAME_LEN {
            return Err(Error::PayloadTooLarge(name_len));
        }
        let device_name = std::str::from_utf8(r.take(name_len)?)?.to_string();
        let jitter_target_ms = r.u16()?;
        Ok(Self {
            status,
            codec,
            sample_rate,
            channels,
            device_name,
            jitter_target_ms,
        })
    }
}

/// 内部用的小端字节游标，带截断检测。
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = match self.pos.checked_add(n) {
            Some(e) => e,
            None => return Err(Error::Truncated { need: usize::MAX, got: self.buf.len() }),
        };
        if end > self.buf.len() {
            return Err(Error::Truncated { need: end, got: self.buf.len() });
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hello() -> Hello {
        Hello {
            device_name: "Pixel 8".into(),
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            buffer_target_ms: 80,
            auth_token_sha256: [0u8; 32],
        }
    }

    #[test]
    fn hello_roundtrip() {
        let h = sample_hello();
        let bytes = h.to_bytes();
        assert_eq!(bytes.len(), 2 + 7 + 1 + 4 + 1 + 2 + 2 + 32);
        assert_eq!(Hello::from_bytes(&bytes).unwrap(), h);
    }

    #[test]
    fn hello_roundtrip_with_token_and_long_name() {
        let mut h = sample_hello();
        h.auth_token_sha256 = core::array::from_fn(|i| i as u8);
        h.device_name = "很长的设备名 ".repeat(30); // 180 字节
        let bytes = h.to_bytes();
        assert_eq!(Hello::from_bytes(&bytes).unwrap(), h);
    }

    #[test]
    fn hello_rejects_truncated() {
        let bytes = sample_hello().to_bytes();
        for cut in [0usize, 1, 5, 40] {
            assert!(Hello::from_bytes(&bytes[..cut.min(bytes.len())]).is_err());
        }
    }

    #[test]
    fn hello_rejects_zero_sample_rate() {
        let mut h = sample_hello();
        h.sample_rate = 0;
        assert!(Hello::from_bytes(&h.to_bytes()).is_err());
    }

    #[test]
    fn hello_ack_roundtrip() {
        let ack = HelloAck {
            status: AckStatus::Ok,
            codec: Codec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            device_name: "DESKTOP-PC".into(),
            jitter_target_ms: 80,
        };
        let bytes = ack.to_bytes();
        assert_eq!(HelloAck::from_bytes(&bytes).unwrap(), ack);
    }

    #[test]
    fn ack_status_roundtrip() {
        for v in 0u8..=5 {
            assert_eq!(AckStatus::from_u8(v).to_u8(), v);
        }
        assert_eq!(AckStatus::from_u8(99), AckStatus::Other(99));
    }
}
