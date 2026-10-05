// release 构建使用 Windows GUI 子系统：启动时不弹出黑色控制台窗口；
// debug 构建保留控制台便于开发调试。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod app;
mod logbuf;
mod settings;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn main() -> eframe::Result<()> {
    let log_buf: logbuf::SharedLog = Arc::new(Mutex::new(VecDeque::new()));
    logbuf::init_logger(Arc::clone(&log_buf));
    log::info!("AudioBridge v{} 启动", env!("CARGO_PKG_VERSION"));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AudioBridge 音频桥")
            .with_inner_size([480.0, 680.0])
            .with_min_inner_size([400.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "AudioBridge",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(app::AudioBridgeApp::new(log_buf)))
        }),
    )
}

/// 加载系统中文字体（微软雅黑等），否则中文会显示为方框。
fn install_cjk_font(ctx: &egui::Context) {
    let candidates = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/msyhl.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".into(), Arc::new(egui::FontData::from_owned(data)));
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "cjk".into());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("cjk".into());
            }
            ctx.set_fonts(fonts);
            log::info!("已加载中文字体: {path}");
            return;
        }
    }
    log::warn!("未找到系统中文字体，中文界面可能显示为方框");
}
