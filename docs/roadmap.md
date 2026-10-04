# 路线图

## v1（当前）

- [x] TCP 协议 v1（HELLO/DATA/PING/BYE，48k/2ch/PCM）
- [x] Windows：WASAPI loopback 采集 + 播放，egui 桌面应用
- [x] Android：MediaProjection 采集 + AudioTrack 播放，Compose 应用
- [x] USB(ADB reverse) 隧道辅助
- [x] Rust/Kotlin 双协议实现 + 字节级一致性测试向量
- [x] 访问令牌（SHA-256 摘要校验）

## v1.x

- [ ] Opus 编码（协议已预留 codec=1）：~64-128kbps，弱 WiFi 更稳
- [ ] 系统托盘图标与最小化（tray-icon）
- [ ] mDNS 局域网自动发现（mdns-sd + NsdManager，`_audiobridge._tcp`）
- [ ] 桌面端选择播放/采集设备（当前固定默认设备）
- [ ] Android 桌面小部件 / 快捷磁贴（TileService）一键开关
- [ ] 断线自动重连与指数退避
- [ ] APK 签名发布流水线 + Windows 安装包（MSIX/zip）

## v2

- [ ] UDP 传输模式 + FEC 前向纠错（弱网/广域网场景）
- [ ] 多接收端同播（一对多）
- [ ] TLS / PSK 加密传输
- [ ] 音频时间戳同步与自适应抖动缓冲（按网络抖动自动调水位）
- [ ] iOS / macOS / Linux 端（core 层已平台无关，需要各平台音频实现）
- [ ] 音频可视化与延迟测量工具（环回打板）

## 已知技术债

- `session.rs` 接收端线程句柄未返回给调用方（靠 stop 标志收敛，正常路径无泄漏，
  但无法 join 确认退出）；
- Kotlin 端 UI 文案为硬编码中文，未走资源国际化；
- 桌面端 USB 按钮依赖 PATH 中的 adb，未内嵌 platform-tools。
