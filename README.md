# AudioBridge 音频桥

把**一台设备的系统声音**通过**局域网或 USB** 实时分享给另一台设备播放。

典型场景：用安卓手机看电视剧，同时在电脑上戴着耳机 —— AudioBridge 让耳机里也
能听到电视剧的声音（反之亦然：电脑的声音也可以发到手机上）。

## 功能特性

- **系统声音采集**（不是麦克风！）：
  - Windows：WASAPI loopback（无需虚拟声卡、无需管理员）
  - Android：MediaProjection + AudioPlaybackCapture（Android 10+，系统弹窗授权）
- **自动发现**：接收端固定端口 48000，发送端一键 UDP 广播扫描局域网，
  点「连接」即用，无需手填 IP（跨路由/AP 隔离网络或 USB 场景仍可手动指定）
- **两个平台互为收发端**：Android→PC、PC→Android、Android→Android、PC→PC 均可
- **传输**：TCP 直连（局域网）；USB 场景经 ADB `reverse` 隧道（同一协议）
- **低延迟**：48kHz 立体声 S16LE + 抖动缓冲，端到端延迟典型 150~300ms（可调）
- **测试源模式**（Android）：不发系统声音、改发内置测试音，用于链路自测与
  ROM 不支持音频回采的设备（如部分模拟器）
- **可调音量**（接收端）、**访问令牌**（可选）、连接状态与丢包/欠载统计
- **一致性测试向量**：Rust 与 Kotlin 双实现共享同一份字节级测试
  （[`protocol/conformance-vectors.json`](protocol/conformance-vectors.json)）

## 组成

| 组件 | 说明 | 技术 |
| --- | --- | --- |
| `crates/core` | 协议、抖动缓冲、会话层（平台无关） | Rust（仅 std） |
| `crates/audio` | WASAPI loopback 采集与播放 | Rust |
| `crates/desktop` | Windows 桌面应用 | Rust + egui |
| `crates/cli` | 命令行收发工具（测试/自动化） | Rust |
| `android/` | Android 应用（发送+接收） | Kotlin + Compose |
| `docs/` | 设计文档、协议规范、构建与使用手册 | Markdown |

## 快速开始

### 场景一：局域网（手机的声音 → 电脑耳机）

1. 电脑端：启动 `audiobridge.exe` → 角色「接收端」→ 开始接收（默认端口 48000）。
2. 手机端：安装 AudioBridge APK → 「发送到电脑」→ 填电脑 IP → 开始发送 →
   在系统弹窗中允许「录制/投射音频」。
3. 电脑端状态变为「已连接」，戴耳机即可听到手机声音。

### 场景二：USB 数据线（更稳、零 WiFi 干扰）

1. 手机开启 USB 调试并连接电脑（`adb` 需在 PATH 中）。
2. 电脑端：角色「发送端」或「接收端」→ 点 **USB(ADB) 接入** 建立隧道。
3. 手机端：目标 IP 填 `127.0.0.1`，端口与电脑一致。

> USB 方向说明：`adb reverse` 把电脑端口映射到手机侧，因此**手机发送、电脑接收**
> 是标准用法。反方向（电脑发送、手机接收）请用局域网。

### 命令行（可选）

```bash
audiobridge-cli listen --port 48000 --sink speaker     # 电脑端接收并播放
audiobridge-cli send   --host 192.168.1.50 --port 48000 # 电脑端采集系统声音发送
audiobridge-cli tone   --freq 440 --seconds 3           # 本机播放测试音
```

## 从源码构建

- Windows：[docs/build-windows.md](docs/build-windows.md)
- Android：[docs/build-android.md](docs/build-android.md)

## 文档索引

- [架构设计](docs/architecture.md) —— 技术选型、模块划分、线程模型、延迟预算
- [线协议规范](docs/protocol.md) —— 字节级协议定义与容错约定
- [使用手册](docs/usage.md) —— 两种场景的详细步骤与故障排查
- [路线图](docs/roadmap.md) —— Opus、UDP 模式、mDNS 发现、系统托盘等

## 已知限制（v1）

- 线协议 v1 固定 48kHz/立体声/PCM，未压缩（约占 1.6 Mbps 带宽，WiFi 可承受）；
  Opus 编码已在协议中预留。
- 局域网延迟约 150~300ms（蓝牙耳机级别），适合观影场景，不适合跟拍节奏的游戏。
- Android 采集端只能采集 `USAGE_MEDIA/GAME/UNKNOWN` 的音频（通话、DRM 保护内容除外）。

## 许可

MIT（见 [LICENSE](LICENSE)）
