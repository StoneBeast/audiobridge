//! 应用内更新：从 GitHub Release 检查新版本 → 应用内下载 → 原子替换 exe → 重启。
//!
//! 清单文件 `update.json` 由 release 工作流生成，固定通过
//! `releases/latest/download/update.json` 访问（无需调用 GitHub API，不受限流）。

use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub const UPDATE_URL: &str =
    "https://github.com/StoneBeast/audiobridge/releases/latest/download/update.json";
pub const RELEASES_PAGE: &str = "https://github.com/StoneBeast/audiobridge/releases/latest";

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub windows_url: String,
    #[serde(default)]
    pub android_url: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// x.y.z 数值比较（忽略 v 前缀与缺失段）。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..3 {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 检查更新；有新版本返回清单，否则 None。
pub fn check() -> Result<Option<UpdateManifest>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build();
    let text = agent
        .get(UPDATE_URL)
        .call()
        .context("无法访问更新服务器（检查网络）")?
        .into_string()?;
    let manifest: UpdateManifest = serde_json::from_str(&text).context("更新清单格式错误")?;
    if is_newer(&manifest.version, current_version()) && !manifest.windows_url.is_empty() {
        Ok(Some(manifest))
    } else {
        Ok(None)
    }
}

/// 流式下载 `url` 到 `dest`（放在与当前 exe 同目录以保证后续可原子重命名）。
/// `progress(got_bytes, total_bytes)`，total 未知时为 0。
pub fn download(url: &str, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .build();
    let resp = agent.get(url).call().context("下载请求失败")?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).with_context(|| format!("无法创建 {}", dest.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut got: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        got += n as u64;
        progress(got, total);
    }
    if got == 0 {
        bail!("下载内容为空");
    }
    Ok(())
}

/// 原子替换当前 exe 并启动新版本（Windows 允许重命名正在运行的 exe）。
pub fn apply_and_restart(new_exe: &Path) -> Result<()> {
    let cur = std::env::current_exe()?;
    let old = cur.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&cur, &old).context("无法重命名当前程序（权限不足？）")?;
    std::fs::rename(new_exe, &cur).context("无法放置新版本文件")?;
    Command::new(&cur).spawn().context("无法启动新版本")?;
    Ok(())
}

/// 用系统浏览器打开 Releases 页面（手动下载兜底）。
pub fn open_releases_page() {
    let _ = Command::new("cmd").args(["/c", "start", "", RELEASES_PAGE]).spawn();
}
