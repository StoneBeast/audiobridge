# AudioBridge 开发环境（本机工具链位于 E:\dev，用户级安装、免管理员）
# 用法: . .\scripts\dev-env.ps1   （或在 PowerShell 中: scripts\dev-env.ps1）

$env:RUSTUP_HOME       = "E:\dev\rust\rustup"
$env:CARGO_HOME        = "E:\dev\rust\cargo"
$env:JAVA_HOME         = "E:\dev\jdk-17"
$env:ANDROID_HOME      = "E:\dev\android-sdk"
$env:GRADLE_USER_HOME  = "E:\dev\.gradle"

$env:PATH = "E:\dev\rust\cargo\bin;" +
            "E:\dev\jdk-17\bin;" +
            "E:\dev\android-sdk\platform-tools;" +
            "E:\dev\android-sdk\cmdline-tools\latest\bin;" +
            "E:\dev\gradle-8.9\bin;" +
            $env:PATH

Write-Host "AudioBridge dev env ready:"
Write-Host "  cargo  = $(cargo --version 2>$null)"
Write-Host "  java   = $($env:JAVA_HOME)"
Write-Host "  sdk    = $($env:ANDROID_HOME)"
