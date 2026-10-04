use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiobridge_audio::{LoopbackSource, SpeakerOutput};
use audiobridge_core::session::{
    run_sender, spawn_receiver, AudioOutput, ReceiveOptions, ReceiverStats, SendOptions,
    SenderStats,
};

use crate::logbuf::SharedLog;
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Receive,
    Send,
}

struct SendSession {
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SenderStats>>,
}

struct RecvSession {
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<ReceiverStats>>,
    volume: Arc<AtomicU32>,
}

pub struct AudioBridgeApp {
    settings: Settings,
    role: Role,
    send: Option<SendSession>,
    recv: Option<RecvSession>,
    last_send: SenderStats,
    last_recv: ReceiverStats,
    volume_ui: f32,
    log: SharedLog,
    status: String,
}

impl AudioBridgeApp {
    pub fn new(log: SharedLog) -> Self {
        let settings = Settings::load();
        let volume_ui = settings.volume_permille as f32 / 10.0;
        Self {
            settings,
            role: Role::Receive,
            send: None,
            recv: None,
            last_send: SenderStats::default(),
            last_recv: ReceiverStats::default(),
            volume_ui,
            log,
            status: String::new(),
        }
    }

    fn start_send(&mut self) {
        if self.send.is_some() {
            return;
        }
        self.settings.save();
        let opts = SendOptions {
            host: self.settings.send_host.trim().to_string(),
            port: self.settings.send_port,
            device_name: self.settings.device_name.clone(),
            frame_ms: 20,
            token: non_empty(&self.settings.token),
        };
        let stats = Arc::new(Mutex::new(SenderStats::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let stats2 = Arc::clone(&stats);
        let stop2 = Arc::clone(&stop);
        let started = std::thread::Builder::new()
            .name("ab-send".into())
            .spawn(move || {
                match LoopbackSource::new(20) {
                    Ok(mut src) => {
                        if let Err(e) = run_sender(&opts, &mut src, stats2, stop2) {
                            log::warn!("sender exited: {e}");
                            upd(&stats2, |s| s.error = Some(format!("{e}")));
                        }
                    }
                    Err(e) => {
                        log::warn!("loopback init failed: {e}");
                        upd(&stats2, |s| {
                            s.error = Some(format!("无法采集系统声音: {e}"))
                        });
                    }
                }
            });
        if started.is_err() {
            self.status = "发送线程启动失败".into();
            return;
        }
        self.send = Some(SendSession { stop, stats });
        self.status = "发送会话启动中…".into();
    }

    fn stop_send(&mut self) {
        if let Some(s) = self.send.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        self.status = "已停止发送".into();
    }

    fn start_recv(&mut self) {
        if self.recv.is_some() {
            return;
        }
        self.settings.save();
        let volume = Arc::new(AtomicU32::new(self.settings.volume_permille));
        let opts = ReceiveOptions {
            port: self.settings.listen_port,
            device_name: self.settings.device_name.clone(),
            frame_ms: 20,
            target_ms: self.settings.target_ms,
            max_ms: 250,
            token: non_empty(&self.settings.token),
            volume_permille: Arc::clone(&volume),
        };
        let stats = Arc::new(Mutex::new(ReceiverStats::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let factory: Box<dyn FnOnce() -> audiobridge_core::Result<Box<dyn AudioOutput>> + Send> =
            Box::new(move || {
                SpeakerOutput::new()
                    .map(|out| Box::new(out) as Box<dyn AudioOutput>)
                    .map_err(|e| audiobridge_core::Error::Io(std::io::Error::other(e)))
            });
        // 若播放设备初始化失败，接收线程会把错误写入 stats 并置位 stop
        let _ = spawn_receiver(opts, Arc::clone(&stats), Arc::clone(&stop), factory);
        self.recv = Some(RecvSession {
            stop,
            stats,
            volume,
        });
        self.status = "接收会话启动中…".into();
    }

    fn stop_recv(&mut self) {
        if let Some(s) = self.recv.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        self.status = "已停止接收".into();
    }

    fn send_pane(&mut self, ui: &mut egui::Ui) {
        let running = self.send.is_some();
        ui.add_space(4.0);
        ui.label("把本机的系统声音发送到另一台设备：");
        ui.add(
            egui::TextEdit::singleline(&mut self.settings.send_host)
                .hint_text("对方 IP；USB 场景填 127.0.0.1")
                .desired_width(260.0),
        );
        ui.horizontal(|ui| {
            ui.label("端口");
            ui.add(egui::DragValue::new(&mut self.settings.send_port).range(1..=65535));
            ui.separator();
            if ui.button("USB(ADB) 接入").clicked() {
                self.settings.save();
                match crate::adb::adb_reverse(self.settings.send_port) {
                    Ok(msg) => {
                        self.settings.send_host = "127.0.0.1".into();
                        self.status = msg;
                    }
                    Err(e) => self.status = format!("USB 失败: {e}"),
                }
            }
        });
        ui.collapsing("高级（访问令牌）", |ui| {
            ui.label("两端设置相同令牌才可连接，留空表示不校验：");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.token)
                    .desired_width(200.0)
                    .password(true),
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !running,
                    egui::Button::new(if running { "发送中…" } else { "开始发送系统声音" }),
                )
                .clicked()
            {
                self.start_send();
            }
            if ui.add_enabled(running, egui::Button::new("停止")).clicked() {
                self.stop_send();
            }
        });
        ui.add_space(8.0);
        let s = self.last_send.clone();
        let (text, color) = if let Some(err) = &s.error {
            (format!("状态：⚠ {err}"), egui::Color32::from_rgb(0xB3, 0x26, 0x1E))
        } else if s.connected {
            (
                format!("状态：已连接 {}", s.peer),
                egui::Color32::from_rgb(0x1B, 0x87, 0x3B),
            )
        } else if running {
            ("状态：连接中…".to_string(), egui::Color32::from_rgb(0x8A, 0x6D, 0x00))
        } else {
            ("状态：未运行".to_string(), egui::Color32::GRAY)
        };
        ui.label(egui::RichText::new(text).color(color));
        if s.connected {
            ui.label(format!(
                "已发送 {}，网络排队丢弃 {} 块",
                bytes_str(s.sent_bytes),
                s.dropped_chunks
            ));
        }
    }

    fn recv_pane(&mut self, ui: &mut egui::Ui) {
        let running = self.recv.is_some();
        ui.add_space(4.0);
        ui.label("接收另一台设备发来的系统声音并播放：");
        ui.horizontal(|ui| {
            ui.label("监听端口");
            ui.add(egui::DragValue::new(&mut self.settings.listen_port).range(1..=65535));
            ui.separator();
            ui.label("缓冲水位(ms)");
            ui.add(egui::DragValue::new(&mut self.settings.target_ms).range(20..=300));
        });
        ui.collapsing("高级（访问令牌）", |ui| {
            ui.label("两端设置相同令牌才可连接，留空表示不校验：");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.token)
                    .desired_width(200.0)
                    .password(true),
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !running,
                    egui::Button::new(if running { "监听中…" } else { "开始接收" }),
                )
                .clicked()
            {
                self.start_recv();
            }
            if ui.add_enabled(running, egui::Button::new("停止")).clicked() {
                self.stop_recv();
            }
        });
        ui.add_space(8.0);
        ui.add(egui::Slider::new(&mut self.volume_ui, 0.0..=100.0).text("音量 %"));
        {
            let Some(session) = &self.recv else { return };
            session
                .volume
                .store((self.volume_ui * 10.0) as u32, Ordering::Relaxed);
        }
        ui.add_space(4.0);
        let s = self.last_recv.clone();
        let (text, color) = if let Some(err) = &s.error {
            (format!("状态：⚠ {err}"), egui::Color32::from_rgb(0xB3, 0x26, 0x1E))
        } else if s.client.is_some() {
            (
                format!("状态：已连接 {}", s.client.clone().unwrap_or_default()),
                egui::Color32::from_rgb(0x1B, 0x87, 0x3B),
            )
        } else if running {
            ("状态：等待连接…".to_string(), egui::Color32::from_rgb(0x8A, 0x6D, 0x00))
        } else {
            ("状态：未运行".to_string(), egui::Color32::GRAY)
        };
        ui.label(egui::RichText::new(text).color(color));
        if running {
            ui.label(format!(
                "缓冲 {}ms，累计收 {}，欠载 {} 次，溢出丢弃 {} B",
                s.buffered_ms,
                bytes_str(s.pushed_bytes),
                s.underruns,
                s.dropped_bytes
            ));
        }
    }

    fn log_pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label("日志");
        egui::ScrollArea::vertical()
            .max_height(110.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if let Ok(g) = self.log.lock() {
                    let skip = g.len().saturating_sub(10);
                    for line in g.iter().skip(skip) {
                        ui.monospace(line);
                    }
                }
            });
    }
}

impl eframe::App for AudioBridgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 会话状态快照（低频轮询即可）
        if let Some(s) = &self.send {
            if let Ok(g) = s.stats.lock() {
                self.last_send = g.clone();
            }
        }
        if let Some(s) = &self.recv {
            if let Ok(g) = s.stats.lock() {
                self.last_recv = g.clone();
            }
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("AudioBridge 音频桥");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("清空日志").clicked() {
                        if let Ok(mut g) = self.log.lock() {
                            g.clear();
                        }
                    }
                });
            });
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("本机角色：");
                ui.radio_value(&mut self.role, Role::Receive, "接收端（本机播放）");
                ui.radio_value(&mut self.role, Role::Send, "发送端（本机采集）");
            });
            ui.add_space(6.0);
            match self.role {
                Role::Receive => self.recv_pane(ui),
                Role::Send => self.send_pane(ui),
            }
            if !self.status.is_empty() {
                ui.separator();
                ui.label(&self.status);
            }
            ui.separator();
            self.log_pane(ui);
        });

        ui.ctx().request_repaint_after(Duration::from_millis(300));
    }
}

impl Drop for AudioBridgeApp {
    fn drop(&mut self) {
        if let Some(s) = self.send.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        if let Some(s) = self.recv.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        self.settings.save();
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

fn upd<T>(m: &Mutex<T>, f: impl FnOnce(&mut T)) {
    if let Ok(mut g) = m.lock() {
        f(&mut g);
    }
}

fn bytes_str(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / 1_048_576.0)
    } else if b >= 1 << 10 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}
