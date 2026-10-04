//! AudioBridge 的 WASAPI 音频实现（仅 Windows）。
//!
//! 线程约定：COM/WASAPI 对象不可跨线程，[`LoopbackSource`] 与
//! [`SpeakerOutput`] 必须在创建它的线程上使用——桌面端/CLI 均在
//! 会话线程内部构造它们。
//!
//! 采集与播放统一使用 48kHz / 双声道 / S16LE；开启 `autoconvert`
//! 让 WASAPI 引擎自动完成采样率/格式转换，无需手动重采样。

mod capture;
mod output;

pub use capture::LoopbackSource;
pub use output::SpeakerOutput;

use wasapi::DeviceEnumerator;

/// 列出可用的播放设备名（供 UI/诊断展示）。
pub fn list_render_devices() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return names;
    };
    let Ok(collection) = enumerator.get_device_collection(&wasapi::Direction::Render) else {
        return names;
    };
    let count = collection.get_nbr_devices().unwrap_or(0);
    for i in 0..count {
        if let Ok(device) = collection.get_device_at_index(i) {
            if let Ok(name) = device.get_friendlyname() {
                names.push(name);
            }
        }
    }
    names
}
