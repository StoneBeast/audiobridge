//! 错误类型。

use thiserror::Error;

/// 握手应答状态（`HELLO_ACK.status`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckStatus {
    /// 接受连接。
    Ok,
    /// 服务端已有活跃会话，拒绝接入。
    Busy,
    /// 协议版本不匹配。
    VersionMismatch,
    /// 请求的音频格式不被支持。
    FormatUnsupported,
    /// 访问令牌校验失败。
    AuthFailed,
    /// 其他错误（保留原始值）。
    Other(u8),
}

impl AckStatus {
    pub const fn to_u8(self) -> u8 {
        match self {
            AckStatus::Ok => 0,
            AckStatus::Busy => 1,
            AckStatus::VersionMismatch => 2,
            AckStatus::FormatUnsupported => 3,
            AckStatus::AuthFailed => 4,
            AckStatus::Other(v) => v,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => AckStatus::Ok,
            1 => AckStatus::Busy,
            2 => AckStatus::VersionMismatch,
            3 => AckStatus::FormatUnsupported,
            4 => AckStatus::AuthFailed,
            other => AckStatus::Other(other),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("bad magic bytes in handshake")]
    BadMagic,
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("unexpected message type: {0} (check connection state)")]
    UnexpectedType(u8),
    #[error("unknown message type: {0}")]
    UnknownType(u8),
    #[error("handshake rejected by peer: {0:?}")]
    Rejected(AckStatus),
    #[error("payload exceeds 65535 bytes: {0}")]
    PayloadTooLarge(usize),
    #[error("message body truncated: need {need} bytes, only {got} available")]
    Truncated { need: usize, got: usize },
    #[error("unknown codec id: {0}")]
    UnknownCodec(u8),
    #[error("invalid field `{0}` in message")]
    InvalidField(&'static str),
    #[error("invalid utf-8 in device name: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
