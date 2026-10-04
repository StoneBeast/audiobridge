//! PCM 采样格式转换与小工具。

/// 交错 f32（[-1,1]）-> 交错 S16LE 字节，带削波。返回写入字节数 = `samples.len() * 2`。
pub fn f32_to_s16_bytes(samples: &[f32], out: &mut [u8]) -> usize {
    let n = samples.len();
    if out.len() < n * 2 {
        return 0;
    }
    for (i, s) in samples.iter().enumerate() {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * 32767.0).round() as i16;
        out[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }
    n * 2
}

/// 交错 S16LE 字节 -> 交错 f32（[-1,1]）。返回写入的采样数；字节不足时按完整帧截断。
pub fn s16_bytes_to_f32(bytes: &[u8], out: &mut [f32]) -> usize {
    let n = (bytes.len() / 2).min(out.len());
    for i in 0..n {
        let lo = bytes[i * 2] as u16 as i16;
        let hi = bytes[i * 2 + 1] as u16 as i16;
        let v = i16::from_le_bytes([lo as u8, hi as u8]);
        out[i] = v as f32 / 32768.0;
    }
    n
}

/// 任意声道 f32 -> 立体声 f32。
///
/// - 1 声道：复制到左右；
/// - 2 声道：原样；
/// - 6 声道（WASAPI 5.1 顺序 FL FR FC LFE BL BR）：简单下混；
/// - 其他：按帧内交错位置平均（偶数位 -> L，奇数位 -> R）。
///
/// v1 的简化实现，供 loopback 采集端使用。
pub fn to_stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 0 {
        return Vec::new();
    }
    match channels {
        1 => {
            let mut out = Vec::with_capacity(samples.len() * 2);
            for &s in samples {
                out.push(s);
                out.push(s);
            }
            out
        }
        2 => samples.to_vec(),
        6 => {
            let frames = samples.len() / 6;
            let mut out = Vec::with_capacity(frames * 2);
            for f in 0..frames {
                let s = &samples[f * 6..f * 6 + 6];
                let l = s[0] + 0.707 * s[2] + 0.707 * s[4];
                let r = s[1] + 0.707 * s[2] + 0.707 * s[5];
                out.push(l.clamp(-1.0, 1.0) * 0.5);
                out.push(r.clamp(-1.0, 1.0) * 0.5);
            }
            out
        }
        ch => {
            let frames = samples.len() / ch;
            let mut out = Vec::with_capacity(frames * 2);
            for f in 0..frames {
                let s = &samples[f * ch..f * ch + ch];
                let mut l = 0.0f32;
                let mut r = 0.0f32;
                let mut nl = 0usize;
                let mut nr = 0usize;
                for (i, &v) in s.iter().enumerate() {
                    if i % 2 == 0 {
                        l += v;
                        nl += 1;
                    } else {
                        r += v;
                        nr += 1;
                    }
                }
                out.push((l / nl.max(1) as f32).clamp(-1.0, 1.0));
                out.push((r / nr.max(1) as f32).clamp(-1.0, 1.0));
            }
            out
        }
    }
}

/// 按千分比音量缩放交错 S16LE 采样（1000 = 原音量）。
pub fn apply_volume_s16(bytes: &mut [u8], permille: u32) {
    if permille >= 1000 {
        return;
    }
    let gain = permille as f32 / 1000.0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        let lo = bytes[i] as u16 as i16;
        let hi = bytes[i + 1] as u16 as i16;
        let v = i16::from_le_bytes([lo as u8, hi as u8]);
        let scaled = (v as f32 * gain).round().clamp(-32768.0, 32767.0) as i16;
        bytes[i..i + 2].copy_from_slice(&scaled.to_le_bytes());
        i += 2;
    }
}

/// 生成正弦波测试音（交错立体声 S16LE 字节），供测试与 CLI 使用。
pub fn sine_s16_stereo(duration_ms: u32, freq_hz: f32, sample_rate: u32, amplitude: f32) -> Vec<u8> {
    let frames = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let v = (2.0 * std::f32::consts::PI * freq_hz * t).sin() * amplitude;
        samples.push(v);
        samples.push(v);
    }
    let mut out = vec![0u8; samples.len() * 2];
    f32_to_s16_bytes(&samples, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_s16_roundtrip() {
        let src: Vec<f32> = [0.0, 0.5, -0.5, 0.999, -1.0, 1.5, -1.5].to_vec();
        let mut bytes = vec![0u8; src.len() * 2];
        assert_eq!(f32_to_s16_bytes(&src, &mut bytes), src.len() * 2);
        let mut back = vec![0f32; src.len()];
        assert_eq!(s16_bytes_to_f32(&bytes, &mut back), src.len());
        assert!((back[0] - 0.0).abs() < 1e-4);
        assert!((back[1] - 0.5).abs() < 1e-4);
        assert!((back[2] + 0.5).abs() < 1e-4);
        assert!((back[5] - 1.0).abs() < 1e-4); // 削波
        assert!((back[6] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn mono_upmix_and_passthrough() {
        assert_eq!(to_stereo(&[0.1, 0.2, 0.3], 1), vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
        assert_eq!(to_stereo(&[0.1, 0.2], 2), vec![0.1, 0.2]);
    }

    #[test]
    fn six_channel_downmix_bounds() {
        let frame: Vec<f32> = vec![0.8, -0.8, 0.5, 0.0, 0.4, -0.4];
        let out = to_stereo(&frame, 6);
        assert_eq!(out.len(), 2);
        for v in out {
            assert!(v >= -1.0 && v <= 1.0);
        }
    }

    #[test]
    fn volume_scaling() {
        let mut bytes = vec![0u8; 4];
        f32_to_s16_bytes(&[0.5, -0.5], &mut bytes); // 16383 / -16384
        apply_volume_s16(&mut bytes, 1000);
        let mut out = [0f32; 2];
        s16_bytes_to_f32(&bytes, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-3); // 1000 不变

        apply_volume_s16(&mut bytes, 500); // 半音量
        s16_bytes_to_f32(&bytes, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-3);
        assert!((out[1] + 0.25).abs() < 1e-3);
    }

    #[test]
    fn sine_shape_and_rms() {
        let bytes = sine_s16_stereo(100, 440.0, 48_000, 0.5);
        assert_eq!(bytes.len(), 192 * 100); // 192B/ms * 100ms
        let mut samples = vec![0f32; bytes.len() / 2];
        s16_bytes_to_f32(&bytes, &mut samples);
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.01);
    }
}
