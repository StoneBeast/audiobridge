//! 音频编码格式与采样参数常量。

/// 线协议 v1 支持的音频负载编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// 交错 S16LE PCM（v1 默认，48kHz 立体声）。
    PcmS16Le,
    /// Opus 编码（预留，v1 未启用）。
    Opus,
}

impl Codec {
    pub const fn to_u8(self) -> u8 {
        match self {
            Codec::PcmS16Le => 0,
            Codec::Opus => 1,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Codec::PcmS16Le),
            1 => Some(Codec::Opus),
            _ => None,
        }
    }
}

/// 统一的线协议采样率。
pub const SAMPLE_RATE: u32 = 48_000;
/// 统一的线协议声道数（立体声）。
pub const CHANNELS: u8 = 2;
/// 默认每个 DATA 帧承载的音频时长（毫秒）。
pub const DEFAULT_FRAME_MS: u16 = 20;
/// 默认接收端抖动缓冲目标水位（毫秒）。50ms 在局域网下延迟/稳定的平衡点；
/// WiFi 恶劣时可调高到 80~150，有线/USB 可低至 20~30。
pub const DEFAULT_TARGET_MS: u16 = 50;
/// 消息负载上限（帧头长度字段为 u16）。
pub const MAX_PAYLOAD: usize = 65_535;
/// 设备名最大字节数（UTF-8）。
pub const MAX_NAME_LEN: usize = 255;

/// S16LE 音频每毫秒的字节数。
pub const fn s16_bytes_per_ms(sample_rate: u32, channels: u8) -> usize {
    (sample_rate as usize * channels as usize * 2) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_roundtrip() {
        for v in 0u8..=1 {
            let c = Codec::from_u8(v).unwrap();
            assert_eq!(c.to_u8(), v);
        }
        assert!(Codec::from_u8(2).is_none());
    }

    #[test]
    fn bytes_per_ms() {
        assert_eq!(s16_bytes_per_ms(48_000, 2), 192);
        assert_eq!(s16_bytes_per_ms(48_000, 1), 96);
        assert_eq!(s16_bytes_per_ms(44_100, 2), 176);
    }
}
