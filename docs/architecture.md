# 架构设计

## 1. 目标与场景

核心需求：设备 A 的**系统输出音频**（不是麦克风）实时传到设备 B 播放；
首发平台 Windows + Android；传输通道局域网或 USB。

倒推出来的关键约束：

1. **采集系统声音在各平台是特权操作**：
   - Windows：WASAPI loopback（共享模式渲染端点的回采，普通用户权限即可）；
   - Android：必须是 MediaProjection + `AudioPlaybackCaptureConfiguration`
     （Android 10+，用户弹窗授权，且必须以前台服务身份运行）。
   两者都没有跨平台库能统一封装，**必须各写原生代码**。
2. **播放**是普通操作：Windows WASAPI 渲染即可；Android `AudioTrack` 即可。
3. **网络与缓冲逻辑**应该只写一份 —— 它是正确性与延迟体验的核心。

## 2. 技术选型

### 2.1 语言与工程结构

| 层 | 选择 | 理由 |
| --- | --- | --- |
| 协议/缓冲/会话逻辑 | **Rust**（`audiobridge-core`） | 无 GC 抖动、纯 std 可在所有平台编译、测试设施完善 |
| Windows 应用 | **Rust + egui** | 单 exe 免安装；WASAPI 采集/播放与核心逻辑同语言无 FFI |
| Android 应用 | **Kotlin + Compose** | MediaProjection/AudioTrack/前台服务都是 JVM API，Kotlin 是唯一顺路的选择 |
| 音频 I/O（Windows） | **wasapi crate** | 支持 loopback；`autoconvert` 让引擎自动做采样率/格式转换 |

**协议实现有两份**（Rust 一份、Kotlin 一份），通过一份字节级一致性测试向量
（`protocol/conformance-vectors.json`）双向锁死兼容性——任何一侧破坏协议
的改动会立刻在 CI 式测试中失败。这是刻意取舍：相比 JNI 桥接 Rust 核心的
构建复杂度（NDK + cargo-ndk + Gradle 集成 + 调试困难），协议本身只有约
400 行，重复实现的风险用测试向量管理更划算。

### 2.2 传输：TCP 而非 UDP（v1）

- 场景是 WiFi/USB **局域网**，丢包率低；TCP 的可靠有序交付让接收端逻辑极简。
- 实时性问题的正确解法不是 UDP，而是「发送端不过度排队 + 接收端丢旧保新」：
  - 发送端 `ChunkQueue` 容量有限，满则丢最旧（不积累延迟）；
  - 接收端抖动缓冲超容量丢最旧，欠载补静音。
- TCP 自己的重传在局域网尾部延迟可控；若未来做广域网/弱网，再加 UDP+序号
  冗余模式（协议已预留 codec/版本字段，见 docs/roadmap.md）。
- USB 场景：`adb reverse tcp:P tcp:P` 把手机侧 `127.0.0.1:P` 映射到 PC 的 P 端口，
  **复用同一条 TCP 协议**，零额外代码。

### 2.3 音频格式：48kHz/立体声/S16LE（v1）

- 48kHz 是消费级设备混音引擎的主流内部采样率（Windows 共享模式混音格式
  绝大多数是 48k float；Android `AudioPlaybackCapture` 请求 48k 无转换）。
- 带宽 48_000 × 2ch × 2B = **192 KB/s ≈ 1.54 Mbps**，WiFi N 起步即可承受。
- Windows 侧开启 WASAPI `AUTOCONVERTPCM` 标志，设备是 44.1k/5.1/float 都由
  引擎转换，应用内不做重采样（省掉 rubato 等依赖）。
- Opus 预留（codec=1）：低延迟+低码率的正确选择，v2 启用。

### 2.4 GUI：egui

- 立即模式 UI，代码量小；`eframe` 0.36 单窗口即可满足设置+状态+日志的需求。
- 中文字体运行时从 `C:\Windows\Fonts\msyh.ttc` 加载（见 `main.rs`）。

## 3. 线协议（详见 docs/protocol.md）

```
发送端(客户端)                                接收端(服务端)
   |--- HELLO(设备名/格式/令牌摘要) ------------->|
   |<------------ HELLO_ACK(接受/拒绝码) ---------|
   |--- DATA(seq, ts, PCM 帧) ------------------>|  ×N
   |--- BYE ------------------------------------>|
```

- 帧长固定头部 + 定长字段，小端；
- `seq` 用于丢包统计；`timestamp_ms` 仅供观测；
- **抖动缓冲不依赖时间戳**：按到达顺序 push/pop，行为只由水位规则决定，
  简单、可测、不受两机时钟偏差影响。

## 4. 核心模块（audiobridge-core）

```
hello.rs     握手消息编解码（字节级，含边界检查）
framing.rs   流式帧读写 + TCP 握手封装
jitter.rs    抖动缓冲：注水→播放→欠载补静音→过载丢最旧
queue.rs     有界块队列（发送端采集→网络解耦，满丢最旧）
pcm.rs       f32↔s16 转换、音量、正弦测试音
session.rs   会话层：AudioSource/AudioOutput 特征 + run_sender/spawn_receiver
```

`session.rs` 把「音频源 → 队列 → 网络」与「网络 → 抖动缓冲 → 输出」的线程
编排独立于平台。平台只需实现两个 trait：

- `AudioSource`（Windows: `LoopbackSource`，测试: `ToneSource`）
- `AudioOutput`（Windows: `SpeakerOutput`，测试: `NullOutput`）

因此**整个会话层可以在无音频设备的 CI 上端到端测试**（tone → TCP → null）。

### 线程模型

发送端：

```
[ab-send 线程]  LoopbackSource.read() -> ChunkQueue ->(Arc)|
[ab-net-send]   ChunkQueue.pop() -> DATA 帧 -> TCP          |
[UI 线程]       轮询 Arc<Mutex<SenderStats>> <--------------+
```

接收端：

```
[ab-listen]  accept（非阻塞轮询，可被 stop 唤醒）-> 每连接 spawn [ab-conn]
[ab-conn]    握手 -> DATA 帧 -> Arc<Mutex<JitterBuffer>>
[ab-playback] jitter.pop() -> 音量 -> SpeakerOutput.write()（事件驱动节拍）
[UI 线程]    轮询 Arc<Mutex<ReceiverStats>>
```

停止语义：一切线程只看 `Arc<AtomicBool>` 停止标志 + 队列 close +
socket shutdown，不使用强杀。

## 5. Android 端结构

```
MainActivity(Compose)  ──角色两个 Tab──┐
   │ MediaProjection 授权弹窗          │
   ▼                                   ▼
CaptureService(前台, mediaProjection)  PlaybackService(前台, mediaPlayback)
   AudioRecord(系统回采48k/s16)         ServerSocket + conn 线程
   StreamSender(TCP 客户端)             JitterBuffer ── AudioTrack
```

- API 34 起要求 `getMediaProjection()` 之前前台服务已运行，授权结果通过
  Intent 传给服务、在 `startForeground()` 后再申请；
- UI 与服务之间用 `AppBus`（`MutableStateFlow`）单向同步；
- 协议/JitterBuffer 在 `proto/` 包，与 Rust 实现共享一致性向量。

## 6. 延迟预算（典型局域网，v0.2 优化后）

| 环节 | 量级 | 说明 |
| --- | --- | --- |
| 采集引擎节拍 | ~10ms | Windows loopback 事件 / Android AudioRecord |
| 发送端帧攒齐(20ms) + TCP | ~25ms | 半帧均值 + 局域网传输 |
| 抖动缓冲水位（默认 50ms） | 50ms | 可调 20~300ms；水位校准防止漂移累积 |
| 播放端缓冲 | ~35ms | WASAPI 缓冲 40ms / Android AudioTrack 60ms |
| **合计** | **≈ 110~150ms** | 优化前 ≈200~300ms |

**水位校准（calibrate）**：收发两端采样时钟存在几十 ppm 偏差，长时间播放
缓冲会单向漂移（越来越延迟或越来越饿）。播放线程每次 pop 后调用校准：
缓冲 > 目标+2 帧则从最旧端跳过（单次限 2 帧），缓冲 ≤ 目标-1 帧则补 1 帧
静音——把延迟**永久钉在目标附近**，音感上只是极罕见的一次轻微跳变。

进一步降低延迟的手段（权衡稳定性）：

1. 接收端「缓冲水位」调到 20~30ms（USB 有线或信号好的 WiFi 下可用）；
2. USB(ADB) 隧道替代 WiFi——排除了无线抖动，欠载率显著下降；
3. v2 启用 Opus + 更短帧（10ms）后，发送侧可再省 ~10ms。

## 7. 安全性

- 明文 PCM 在局域网内传输；可选「访问令牌」：HELLO 携带 SHA-256(令牌)，
  接收端比对（防随手蹭连，不防有意嗅探）；
- 接收端同一时刻只接受一路流，其余连接收到 `BUSY`；
- v2 可升级 TLS 或 PSK 加密（协议版本字段已预留）。
