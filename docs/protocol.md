# AudioBridge 线协议规范（v1）

> 本文档是 AudioBridge 网络协议的**权威规范**。Rust 实现（`audiobridge-core`）
> 与 Kotlin 实现（Android 端 `proto/` 包）都必须遵守本文。
> 一致性测试向量：[`protocol/conformance-vectors.json`](../protocol/conformance-vectors.json)，
> 两侧的自动化测试都会用同一份向量做字节级校验。

## 1. 总览

- 传输层：TCP（局域网直连；USB 场景经 ADB `reverse` 隧道，仍是 TCP）。
- 字节序：全部小端（little-endian）。
- 角色约定：**音频发送端是 TCP 客户端，接收端是 TCP 服务端**。
  双向分享 = 两个方向的连接各建一条。
- 连接模型：接收端同一时刻只接受一条活跃流；新连接到达时以 `BUSY` 拒绝。
- 音频参数（v1 固定）：48 kHz、2 声道（立体声）、交错 S16LE PCM；
  Opus 编码已在协议中预留（`codec=1`），实现待后续版本启用。

## 2. 握手

连接建立后**必须首先**进行如下握手，然后才能发送任何其他消息。

```
偏移  长度  字段
0     4     魔数 "ABRG" (0x41 0x42 0x52 0x47)
4     1     协议版本，当前 = 1
5     1     消息类型：1 = HELLO（客户端发），2 = HELLO_ACK（服务端回）
6     2     消息体长度 N (u16)
8     N     消息体
```

### 2.1 HELLO（消息体，发送端 -> 接收端）

```
偏移  长度  字段                       说明
0     2     name_len (u16)             设备名字节数，<= 255
2     name_len  device_name             UTF-8
..    1     codec                      0 = PCM_S16LE, 1 = OPUS
..    4     sample_rate (u32)          线协议采样率，如 48000
..    1     channels                   1..=8，v1 约定 2
..    2     frame_ms (u16)             每帧标称时长，如 20
..    2     buffer_target_ms (u16)     建议接收端抖动缓冲水位，如 80
..    32    token_sha256               访问令牌的 SHA-256；全零 = 不校验
```

### 2.2 HELLO_ACK（消息体，接收端 -> 发送端）

```
偏移  长度  字段                       说明
0     1     status                     0=OK 1=BUSY 2=VERSION 3=FORMAT 4=AUTH >=5=OTHER
1     1     codec                      协商后的编码
2     4     sample_rate (u32)          协商后的采样率
6     1     channels                   协商后的声道数
7     2     name_len (u16)             接收端设备名字节数，<= 255
9     name_len  device_name            UTF-8
..    2     jitter_target_ms (u16)     接收端实际使用的抖动缓冲水位
```

约定：

- `status != 0` 时接收端发送 HELLO_ACK 后应关闭连接；发送端不得继续发送数据。
- 任何一方收到未知 `codec`、`status` 值都应视为协议错误并关闭连接（未知
  status 数值可透传为 OTHER）。
- 版本协商：v1 只接受 `version == 1`；收到其他版本回
  `HELLO_ACK{status=VERSION}`（若能解析）或直接断开。

## 3. 流式消息（握手完成后，双向）

```
偏移  长度  字段
0     1     msg_type：3=DATA 4=PING 5=PONG 6=BYE
1     2     payload_len (u16)
3     N     payload
```

### 3.1 DATA（payload，发送端 -> 接收端）

```
偏移  长度  字段
0     4     seq (u32)          发送端单调递增序号（允许回绕）
4     4     timestamp_ms (u32) 发送端单调时钟毫秒（mod 2^32），仅统计用
8     ..    audio              音频负载（编码见握手），PCM 时为交错 S16LE
```

- `audio` 长度约定：PCM 时 `frame_ms * sample_rate * channels * 2 / 1000`
  字节（48kHz/2ch/20ms = 3840 字节），允许尾帧变短（停止前）。
- 接收端**必须容忍**任意负载长度（按字节流写入抖动缓冲）。

### 3.2 PING / PONG（双向）

`payload` 为 4 字节 nonce；收到 PING 应回 PONG 并携带相同 nonce。
长连接空闲检测用，属可选实现。

### 3.3 BYE（双向）

`payload` 为 `[reason u8][可选 UTF-8 说明]`：
`0=正常结束 1=本端退出 2=内部错误`。发送 BYE 后关闭 TCP。

## 4. 容错与行为约定

| 情形 | 约定行为 |
| --- | --- |
| 收到错误魔数 | 立即断开 |
| 收到 `version != 1` | 回 BUSY/VERSION 或断开 |
| 负载超过声明的 `payload_len` | 不可能（按长度读）；长度字段本身上限 65535 |
| DATA 间网络中断 | 接收端回到监听状态，UI 提示断线 |
| 接收端缓冲溢出 | 丢弃最旧数据，统计 `dropped` |
| 接收端缓冲耗尽 | 播放静音，统计 `underruns`，数据恢复后立即续播 |

## 5. 一致性测试

- Rust：`cargo test -p audiobridge-core`（校验实现 == 提交的向量）。
- 重新生成向量：`AB_GENERATE_VECTORS=1 cargo test -p audiobridge-core`
  （**必须同步评审协议文档变更**）。
- Kotlin：`./gradlew :app:testDebugUnitTest` 读取同一 JSON，做
  解码（字节 -> 字段）与编码（字段 -> 字节）双向校验。
