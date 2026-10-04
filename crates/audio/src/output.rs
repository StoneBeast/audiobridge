//! 系统默认播放设备输出（WASAPI 共享模式，事件驱动）。

use std::io;

use audiobridge_core::session::AudioOutput;
use wasapi::{
    initialize_mta, AudioClient, AudioRenderClient, Direction, DeviceEnumerator, Handle, SampleType,
    StreamMode, WaveFormat,
};

/// 把抖动缓冲取出的 S16LE 数据写到默认播放设备。
///
/// `write` 阻塞到数据被设备缓冲接收为止，节奏由事件驱动；
/// 设备长时间无响应（拔出/独占）时返回 Err 让会话退出。
pub struct SpeakerOutput {
    audio_client: AudioClient,
    render_client: AudioRenderClient,
    event: Handle,
    blockalign: usize,
    stalls: u32,
}

/// 播放缓冲时长（100ns 单位），80ms —— 延迟与稳定性的折中。
const BUFFER_DURATION_HNS: i64 = 800_000;
const MAX_STALLS: u32 = 30; // 30 * 100ms ≈ 3s 无响应则报错

impl SpeakerOutput {
    pub fn new() -> Result<Self, wasapi::WasapiError> {
        Self::new_with_format(WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None))
    }

    pub fn new_with_format(format: WaveFormat) -> Result<Self, wasapi::WasapiError> {
        initialize_mta().ok()?;
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut audio_client = device.get_iaudioclient()?;
        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: BUFFER_DURATION_HNS,
        };
        audio_client.initialize_client(&format, &Direction::Render, &mode)?;
        let event = audio_client.set_get_eventhandle()?;
        let render_client = audio_client.get_audiorenderclient()?;
        let buffer_frames = audio_client.get_buffer_size()?;
        let blockalign = format.get_blockalign() as usize;
        audio_client.start_stream()?;

        // 预填 3/4 静音，避免启动瞬间播放未定义数据
        let prime_frames = buffer_frames as usize * 3 / 4;
        let zeros = vec![0u8; prime_frames * blockalign];
        render_client.write_to_device(prime_frames, &zeros, None)?;

        log::info!(
            "speaker output started (buffer {} frames, blockalign {} B)",
            buffer_frames,
            blockalign
        );
        Ok(Self {
            audio_client,
            render_client,
            event,
            blockalign,
            stalls: 0,
        })
    }
}

impl Drop for SpeakerOutput {
    fn drop(&mut self) {
        let _ = self.audio_client.stop_stream();
        log::info!("speaker output stopped");
    }
}

impl AudioOutput for SpeakerOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        let frames = buf.len() / self.blockalign;
        if frames == 0 {
            return Ok(());
        }
        loop {
            let space = self.audio_client.get_available_space_in_frames()? as usize;
            if space >= frames {
                return self
                    .render_client
                    .write_to_device(frames, buf, None)
                    .map_err(|e| io::Error::other(e));
            }
            match self.event.wait_for_event(100) {
                Ok(()) => {
                    self.stalls = 0;
                }
                Err(_) => {
                    self.stalls += 1;
                    if self.stalls > MAX_STALLS {
                        return Err(io::Error::other("播放设备长时间无响应"));
                    }
                }
            }
        }
    }
}
