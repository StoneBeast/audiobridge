@echo off
rem AudioBridge dev environment (toolchains live on E:\dev, user-level install)
set "RUSTUP_HOME=E:\dev\rust\rustup"
set "CARGO_HOME=E:\dev\rust\cargo"
set "JAVA_HOME=E:\dev\jdk-17"
set "ANDROID_HOME=E:\dev\android-sdk"
set "GRADLE_USER_HOME=E:\dev\.gradle"
set "PATH=E:\dev\rust\cargo\bin;E:\dev\jdk-17\bin;E:\dev\android-sdk\platform-tools;E:\dev\android-sdk\cmdline-tools\latest\bin;E:\dev\gradle-8.9\bin;%PATH%"
echo AudioBridge dev env ready.
