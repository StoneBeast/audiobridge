# Android 端构建指南

## 1. 需要的环境

| 工具 | 本仓库使用的版本 | 说明 |
| --- | --- | --- |
| JDK | Temurin 17（`E:\dev\jdk-17`） | AGP 8.5 要求 17+ |
| Android SDK | platform-tools / platforms;android-34 / build-tools;34.0.0 | cmdline-tools 安装 |
| Gradle | 8.9（仓库已带 wrapper） | 首次构建自动下载依赖 |

本仓库开发机把工具链统一放在 `E:\dev`（无管理员权限、全部用户级安装）：

```
E:\dev\jdk-17\            JDK 17
E:\dev\android-sdk\       Android SDK
E:\dev\.gradle\           GRADLE_USER_HOME（依赖缓存）
```

## 2. 环境变量（或运行 `scripts/dev-env.ps1`）

```powershell
$env:JAVA_HOME = "E:\dev\jdk-17"
$env:ANDROID_HOME = "E:\dev\android-sdk"
$env:GRADLE_USER_HOME = "E:\dev\.gradle"
$env:PATH = "$env:JAVA_HOME\bin;$env:ANDROID_HOME\platform-tools;$env:PATH"
```

`android/local.properties`（已被 gitignore）：

```
sdk.dir=E\:\\dev\\android-sdk
```

## 3. 构建

```bash
cd android

# 单元测试（含与 Rust 共享的协议一致性向量测试）
gradlew.bat :app:testDebugUnitTest

# Debug APK
gradlew.bat :app:assembleDebug
# 产物: app/build/outputs/apk/debug/app-debug.apk
```

安装到手机：

```bash
adb install -r app\build\outputs\apk\debug\app-debug.apk
```

## 4. Release 签名（可选）

```powershell
# 一次性生成签名密钥（密码示例 audiobridge，请自行更换）
keytool -genkeypair -v -keystore android\keystore\release.jks `
  -alias audiobridge -keyalg RSA -keysize 2048 -validity 10000 `
  -storepass audiobridge -keypass audiobridge -dname "CN=AudioBridge"

# 构建（build.gradle.kts 检测到 keystore/release.jks 即自动签名）
gradlew.bat :app:assembleRelease
```

`keystore/*.jks` 已被 gitignore；换机器重新生成即可（升级安装需要同一密钥）。

## 5. 系统要求与权限

- **Android 10 (API 29)+**：系统音频捕获 `AudioPlaybackCapture` 的最低版本；
- 首次「开始发送」会弹出系统「录制/投射音频」授权，拒绝则无法采集；
- Android 13+ 需允许通知权限（前台服务通知）；
- 采集范围：`USAGE_MEDIA / GAME / UNKNOWN` 的音频流（即多数应用的多媒体
  声音）；通话音频与 DRM 保护内容不在可捕获范围（系统限制）。

## 6. 常见问题

**Q: gradlew 报 `SDK location not found`**
确认 `android/local.properties` 存在且 `sdk.dir` 指向正确，或设置 `ANDROID_HOME`。

**Q: 依赖下载慢/失败**
Gradle 走 `GRADLE_USER_HOME`；如需代理，在 `E:\dev\.gradle\gradle.properties`
配置 `systemProp.http.proxyHost` 等。

**Q: 发送端连接成功但没有声音**
手机媒体音量、接收端音量、以及是否被其他应用抢占；另见 docs/usage.md 故障排查。
