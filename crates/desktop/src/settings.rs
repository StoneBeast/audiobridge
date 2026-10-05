use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 桌面端设置（持久化到 %LOCALAPPDATA%/audiobridge/settings.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 发送模式目标主机。
    pub send_host: String,
    /// 发送模式目标端口。
    pub send_port: u16,
    /// 接收模式监听端口。
    pub listen_port: u16,
    /// 接收端抖动缓冲目标水位（毫秒）。
    pub target_ms: u16,
    /// 可选访问令牌（两端一致才能连接）。
    pub token: String,
    /// 播放音量（千分比）。
    pub volume_permille: u32,
    /// 本机显示名。
    pub device_name: String,
    /// 用户选择「忽略」的更新版本号（该版本不再提示）。
    pub ignored_update_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            send_host: "192.168.1.100".into(),
            send_port: 48_000,
            listen_port: 48_000,
            target_ms: audiobridge_core::codec::DEFAULT_TARGET_MS,
            token: String::new(),
            volume_permille: 1000,
            device_name: "Windows PC".into(),
            ignored_update_version: String::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn path() -> Option<PathBuf> {
        dirs::config_local_dir().map(|d| d.join("audiobridge").join("settings.json"))
    }
}
