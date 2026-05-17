use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use log::{Level, Log, Metadata, Record};

const MAX_ENTRIES: usize = 2000;

static COLLECTOR: OnceLock<LogCollector> = OnceLock::new();

pub struct LogEntry {
    pub level: Level,
    pub target: String,
    pub message: String,
    pub timestamp: Instant,
}

struct LogCollector {
    entries: Mutex<VecDeque<LogEntry>>,
    startup: Instant,
}

impl Log for LogCollector {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let entry = LogEntry {
            level: record.level(),
            target: record.target().to_string(),
            message: format!("{}", record.args()),
            timestamp: Instant::now(),
        };
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= MAX_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(entry);
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    let collector = COLLECTOR.get_or_init(|| LogCollector {
        entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
        startup: Instant::now(),
    });
    let _ = log::set_logger(collector);
    log::set_max_level(log::LevelFilter::Debug);
}

pub fn entries_snapshot() -> Vec<(Level, String, String, f64)> {
    let Some(collector) = COLLECTOR.get() else {
        return Vec::new();
    };
    let Ok(entries) = collector.entries.lock() else {
        return Vec::new();
    };
    let startup = collector.startup;
    entries
        .iter()
        .map(|e| {
            let elapsed = e.timestamp.duration_since(startup).as_secs_f64();
            (e.level, e.target.clone(), e.message.clone(), elapsed)
        })
        .collect()
}
