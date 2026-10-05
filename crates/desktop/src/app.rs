use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiobridge_audio::{LoopbackSource, SpeakerOutput};
use audiobridge_core::discovery::{self, DiscoveredDevice};
use audiobridge_core::session::{
    run_sender, spawn_receiver, AudioOutput, ReceiveOptions, ReceiverStats, SendOptions,
    SenderStats,
};

use crate::logbuf::SharedLog;
use crate::settings::Settings;
use crate::updater;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Receive,
    Send,
}

/// 应用内更新的共享状态（后台线程写、UI 线程读）。
#[derive(Default)]
struct UpdateShared {
    info: Option<updater::UpdateManifest>,
    /// 手动检查的结果提示（"检查更新中…" / "已是最新版本…" / 失败原因）。
    status: String,
    status_at: Option<std::time::Instant>,
    downloading: bool,
    got: u64,
    total: u64,
    /// 已下载完成、等待用户重启安装的新版本文件。
    ready: Option<std::path::PathBuf>,
    error: Option<String>,
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
    scan_devices: Arc<Mutex<Vec<DiscoveredDevice>>>,
    scanning: Arc<AtomicBool>,
    update: Arc<Mutex<UpdateShared>>,
    log: SharedLog,
    status: String,
}

impl AudioBridgeApp {
    pub fn new(log: SharedLog) -> Self {
        let settings = Settings::load();
        let volume_ui = settings.volume_permille as f32 / 10.0;
        let update: Arc<Mutex<UpdateShared>> = Arc::new(Mutex::new(UpdateShared::default()));
        let app = Self {
            settings,
            role: Role::Receive,
            send: None,
            recv: None,
            last_send: SenderStats::default(),
            last_recv: ReceiverStats::default(),
            volume_ui,
            scan_devices: Arc::new(Mutex::new(Vec::new())),
            scanning: Arc::new(AtomicBool::new(false)),
            update,
            log,
            status: String::new(),
        };
        app.spawn_update_check(false);
        app
    }

    /// 检查更新（启动时自动 + 标题栏手动）。
    /// 手动检查时若发现新版本，清除用户之前「忽略」的版本（明确想看）。
    fn spawn_update_check(&self, manual: bool) {
        let upd = Arc::clone(&self.update);
        {
            let mut g = upd.lock().unwrap();
            g.status = "检查更新中…".into();
            g.status_at = Some(std::time::Instant::now());
        }
        std::thread::Builder::new()
            .name("ab-update-check".into())
            .spawn(move || match updater::check() {
                Ok(Some(m)) => {
                    let mut g = upd.lock().unwrap();
                    if manual {
                        g.status.clear();
                    }
                    g.info = Some(m);
                }
                Ok(None) => {
                    let mut g = upd.lock().unwrap();
                    g.info = None;
                    g.status = format!("已是最新版本 v{}", updater::current_version());
                    g.status_at = Some(std::time::Instant::now());
                }
                Err(e) => {
                    let mut g = upd.lock().unwrap();
                    g.status = format!("检查更新失败: {e}");
                    g.status_at = Some(std::time::Instant::now());
                }
            })
            .ok();
    }

    fn start_update_download(&mut self) {
        let url = {
            let g = self.update.lock().unwrap();
            match &g.info {
                Some(i) => i.windows_url.clone(),
                None => return,
            }
        };
        self.update.lock().unwrap().downloading = true;
        let upd = Arc::clone(&self.update);
        std::thread::Builder::new()
            .name("ab-update-dl".into())
            .spawn(move || {
                let dest = match std::env::current_exe() {
                    Ok(p) => p.with_file_name("audiobridge.exe.new"),
                    Err(e) => {
                        let mut g = upd.lock().unwrap();
                        g.downloading = false;
                        g.error = Some(format!("{e}"));
                        return;
                    }
                };
                let r = updater::download(&url, &dest, &|got, total| {
                    let mut g = upd.lock().unwrap();
                    g.got = got;
                    g.total = total;
                });
                match r {
                    Ok(()) => {
                        // 后台下载完成：不自动重启，横幅提示「重启并安装」；
                        // 用户直接关闭程序时 Drop 也会完成替换
                        let mut g = upd.lock().unwrap();
                        g.downloading = false;
                        g.ready = Some(dest);
                    }
                    Err(e) => {
                        let mut g = upd.lock().unwrap();
                        g.downloading = false;
                        g.error = Some(format!("{e}"));
                    }
                }
            })
            .ok();
    }

    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let snapshot = {
            let g = self.update.lock().unwrap();
            (
                g.info.clone(),
                g.downloading,
                g.got,
                g.total,
                g.ready.clone(),
                g.error.clone(),
            )
        };
        let (info, downloading, got, total, ready, error) = snapshot;
        let Some(ref m) = info else {
            return;
        };
        // 用户忽略过的版本不再自动提示（手动检查会清除忽略）
        if m.version == self.settings.ignored_update_version {
            return;
        }

        egui::Frame::default()
            .fill(egui::Color32::from_rgb(0xFF, 0xF3, 0xD6))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let Some(ref path) = ready {
                        ui.label(
                            egui::RichText::new(format!(
                                "新版本 v{} 已下载完成（当前 v{}）",
                                m.version,
                                updater::current_version()
                            ))
                            .strong(),
                        );
                        if ui.button("重启并安装").clicked() {
                            match updater::apply_and_restart(path) {
                                Ok(()) => std::process::exit(0),
                                Err(e) => {
                                    self.update.lock().unwrap().error =
                                        Some(format!("替换程序失败: {e}（可到 Releases 页手动下载）"));
                                }
                            }
                        }
                        ui.weak("（直接关闭程序也会在退出时完成更新）");
                    } else if downloading {
                        ui.label("正在后台下载更新…");
                        let frac = if total > 0 { got as f32 / total as f32 } else { 0.0 };
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .show_percentage()
                                .desired_width(140.0),
                        );
                        ui.weak("（可继续使用，完成后在此提示）");
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "发现新版本 v{}（当前 v{}）",
                                m.version,
                                updater::current_version()
                            ))
                            .strong(),
                        );
                        if ui.button("更新").clicked() {
                            self.start_update_download();
                        }
                        if ui.small_button("忽略此版本").clicked() {
                            self.settings.ignored_update_version = m.version.clone();
                            self.settings.save();
                        }
                    }
                });
                if !m.notes.is_empty() {
                    ui.label(egui::RichText::new(&m.notes).small());
                }
                if let Some(err) = &error {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("更新失败: {err}"))
                            .color(egui::Color32::from_rgb(0xB3, 0x26, 0x1E)));
                        if ui.small_button("打开下载页").clicked() {
                            updater::open_releases_page();
                        }
                    });
                }
            });
        ui.add_space(4.0);
    }

    fn start_scan(&mut self) {
        if self.scanning.load(Ordering::Relaxed) {
            return;
        }
        self.scanning.store(true, Ordering::Relaxed);
        if let Ok(mut g) = self.scan_devices.lock() {
            g.clear();
        }
        let port = self.settings.send_port;
        let devices = Arc::clone(&self.scan_devices);
        let scanning = Arc::clone(&self.scanning);
        std::thread::Builder::new()
            .name("ab-scan".into())
            .spawn(move || match discovery::scan(port, 2500) {
                Ok(found) => {
                    log::info!("扫描完成：发现 {} 台设备", found.len());
                    if let Ok(mut g) = devices.lock() {
                        *g = found;
                    }
                    scanning.store(false, Ordering::Relaxed);
                }
                Err(e) => {
                    log::warn!("扫描失败: {e}");
                    scanning.store(false, Ordering::Relaxed);
                }
            })
            .ok();
        self.status = format!("正在扫描 udp/{port}（约 2.5 秒）…");
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
                        if let Err(e) = run_sender(&opts, &mut src, Arc::clone(&stats2), Arc::clone(&stop2)) {
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
        // 局域网自动发现：应答 UDP 探测（与 TCP 同端口）
        discovery::spawn_responder(
            self.settings.listen_port,
            self.settings.listen_port,
            self.settings.device_name.clone(),
            Arc::clone(&stop),
        );
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

        // —— 自动发现 ——
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let scanning = self.scanning.load(Ordering::Relaxed);
            if ui
                .add_enabled(!scanning, egui::Button::new(if scanning { "扫描中…" } else { "⟳ 扫描局域网设备" }))
                .clicked()
            {
                self.start_scan();
            }
            let n = self.scan_devices.lock().map(|g| g.len()).unwrap_or(0);
            ui.label(format!("发现 {n} 台"));
        });
        let devices = self.scan_devices.lock().map(|g| g.clone()).unwrap_or_default();
        for d in &devices {
            ui.horizontal(|ui| {
                ui.label(format!("{}（{}）", d.name, d.addr));
                if ui
                    .add_enabled(!running, egui::Button::new("连接"))
                    .clicked()
                {
                    self.settings.send_host = d.addr.to_string();
                    self.settings.send_port = d.tcp_port;
                    self.settings.save();
                    self.start_send();
                }
            });
        }
        if devices.is_empty() && !self.scanning.load(Ordering::Relaxed) {
            ui.weak("未发现设备：请确认对方已选「接收端」并启动监听");
        }

        // —— 手动地址（高级）——
        ui.collapsing("手动指定 IP / USB(ADB)（高级）", |ui| {
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
        ui.label("接收另一台设备发来的系统声音并播放（手机端点「扫描局域网设备」即可找到本机）：");
        ui.collapsing("高级（端口 / 缓冲 / 令牌）", |ui| {
            ui.horizontal(|ui| {
                ui.label("监听端口");
                ui.add(egui::DragValue::new(&mut self.settings.listen_port).range(1..=65535));
                ui.separator();
                ui.label("缓冲水位(ms)");
                ui.add(egui::DragValue::new(&mut self.settings.target_ms).range(20..=300));
            });
            ui.label("访问令牌（两端一致才可连接，留空不校验）：");
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
            if s.client.is_none() && s.error.is_none() {
                ui.weak("提示：手机连不上时——①首次监听的 Windows 防火墙弹窗要点「允许」（错过请到防火墙放行本程序或 48000 端口）；②手机端重新扫描或核对 IP；③USB 场景先点发送端「USB(ADB) 接入」");
            }
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
                    if ui.small_button("检查更新").clicked() {
                        self.spawn_update_check(true);
                    }
                    if ui.small_button("清空日志").clicked() {
                        if let Ok(mut g) = self.log.lock() {
                            g.clear();
                        }
                    }
                    // 手动检查的反馈（6 秒后淡出，避免常驻噪音）
                    let show_status = {
                        let g = self.update.lock().unwrap();
                        !g.status.is_empty()
                            && g.status_at
                                .map(|t| t.elapsed() < Duration::from_secs(6))
                                .unwrap_or(false)
                    };
                    if show_status {
                        let s = self.update.lock().unwrap().status.clone();
                        ui.label(egui::RichText::new(&s).small().weak());
                    }
                });
            });
            self.update_banner(ui);
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
        // 已后台下载完成、尚未安装的更新：退出时完成替换，下次启动即新版本
        if let Ok(g) = self.update.lock() {
            if let Some(path) = &g.ready {
                if let Err(e) = updater::apply_update_only(path) {
                    log::warn!("退出时应用更新失败: {e}");
                } else {
                    log::info!("更新已在退出时应用，下次启动为新版本");
                }
            }
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
