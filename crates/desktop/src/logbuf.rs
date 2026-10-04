use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Once};

pub type SharedLog = Arc<Mutex<VecDeque<String>>>;

/// 安装环形缓冲日志器（GUI 内展示最近 200 条，同时输出到 stderr）。
pub fn init_logger(buf: SharedLog) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = log::set_boxed_logger(Box::new(RingLogger { buf }));
        log::set_max_level(log::LevelFilter::Info);
    });
}

struct RingLogger {
    buf: SharedLog,
}

impl log::Log for RingLogger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("[{}] {}", short_level(record.level()), record.args());
        eprintln!("{line}");
        if let Ok(mut g) = self.buf.lock() {
            g.push_back(line);
            while g.len() > 200 {
                g.pop_front();
            }
        }
    }

    fn flush(&self) {}
}

fn short_level(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "E",
        log::Level::Warn => "W",
        log::Level::Info => "I",
        log::Level::Debug => "D",
        log::Level::Trace => "T",
    }
}
