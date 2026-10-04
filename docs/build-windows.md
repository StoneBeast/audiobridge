# Windows 端构建指南

## 1. 需要的环境

| 工具 | 本仓库使用的版本 | 说明 |
| --- | --- | --- |
| Rust (GNU 工具链) | stable-x86_64-pc-windows-gnu | 自带 MinGW 链接器，**无需安装 Visual Studio/MSVC** |
| Git | 任意较新版本 | |
| （可选）adb | platform-tools | 仅 USB 场景需要，加入 PATH |

> 为什么用 GNU 工具链：MSVC 链接器依赖 Visual Studio Build Tools（需要管理员
> 权限安装，体积 4GB+）。windows-gnu 工具链由 rustup 自带 MinGW，解压即用。
> 本项目的依赖（windows/wasapi/egui 等）均官方支持 windows-gnu 目标。

## 2. 安装 Rust（免管理员，装到 E 盘示例）

```powershell
$env:RUSTUP_HOME = "E:\dev\rust\rustup"
$env:CARGO_HOME   = "E:\dev\rust\cargo"
# 下载 rustup-init.exe（https://win.rustup.rs/x86_64）后：
.\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --default-toolchain stable --profile minimal --no-modify-path
```

之后每个开发终端都需要：

```powershell
$env:RUSTUP_HOME = "E:\dev\rust\rustup"
$env:CARGO_HOME   = "E:\dev\rust\cargo"
$env:PATH = "E:\dev\rust\cargo\bin;$env:PATH"
```

（或直接运行仓库脚本 `scripts/dev-env.ps1`。）

## 3. 构建与测试

仓库根目录（含 `Cargo.toml` workspace）：

```bash
# 全部单元测试 + 集成测试（不依赖音频设备）
cargo test

# 桌面应用（target/release/audiobridge.exe）
cargo build --release -p audiobridge-desktop

# 命令行工具（target/release/audiobridge-cli.exe）
cargo build --release -p audiobridge-cli
```

> 首次构建需下载依赖（eframe/egui 约 40+ crate），之后增量构建很快。

### 一致性测试向量

协议字节编码由 `protocol/conformance-vectors.json` 锁定。修改协议后重新生成：

```bash
AB_GENERATE_VECTORS=1 cargo test -p audiobridge-core
# 同时必须同步更新 docs/protocol.md，并确认 Android 端测试也通过
```

## 4. 产物

| 文件 | 用途 |
| --- | --- |
| `target/release/audiobridge.exe` | 桌面 GUI（release 构建隐藏控制台窗口） |
| `target/release/audiobridge-cli.exe` | 命令行收发工具 |

把 exe 拷贝到任意目录即可运行，无需安装。

## 5. 常见问题

**Q: 链接报 `undefined reference to ...`**
确认使用的是 `x86_64-pc-windows-gnu` 工具链（`rustc -vV` 查看 host），
且没有混用 MSVC 目标产物（删除 `target/` 重新构建）。

**Q: egui 界面中文显示为方框**
字体加载依赖 `C:\Windows\Fonts\msyh.ttc`（Windows 10/11 自带）；
自定义精简系统需补装微软雅黑。

**Q: 采集不到系统声音 / 状态报「无法采集系统声音」**
- 确认本机有可用的默认播放设备（WASAPI loopback 依附默认渲染端点）；
- 独占模式占用声卡的播放器会导致共享模式回采失败，关闭后重试。
