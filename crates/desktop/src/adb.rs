//! USB(ADB) 隧道辅助：`adb reverse` 把 PC 的端口映射到手机的 127.0.0.1。
//!
//! 原理：Android 采集端连接 `127.0.0.1:PORT` 时，ADB 会把该连接经 USB
//! 转发到 PC 侧的 `PORT`，与局域网 TCP 完全同一条协议通道。

use std::process::Command;

use anyhow::{bail, Context, Result};

/// 检查是否有已授权的 USB 设备，返回数量。
pub fn adb_devices() -> Result<usize> {
    let out = Command::new("adb")
        .arg("devices")
        .output()
        .context("找不到 adb 命令（请安装 Android platform-tools 并加入 PATH）")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let n = text
        .lines()
        .skip(1)
        .filter(|l| l.trim().ends_with("device"))
        .count();
    if n == 0 {
        bail!("未检测到 USB 连接的 Android 设备（需在手机上开启 USB 调试并授权）");
    }
    Ok(n)
}

/// 建立 `adb reverse tcp:port tcp:port` 隧道，返回提示信息。
pub fn adb_reverse(port: u16) -> Result<String> {
    adb_devices()?;
    let out = Command::new("adb")
        .args(["reverse", &format!("tcp:{port}"), &format!("tcp:{port}")])
        .output()
        .context("执行 adb 失败")?;
    if !out.status.success() {
        bail!(
            "adb reverse 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(format!(
        "USB 隧道已建立：手机端「发送到电脑」目标填 127.0.0.1:{port}"
    ))
}
