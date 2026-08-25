//! Persistent shell logger.
//!
//! Everything diagnostic goes BOTH to stdout (visible in terminal runs)
//! AND to `<app_data>/logs/shell.log`, so background/double-clicked
//! launches remain fully debuggable without redirect gymnastics.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = LOG_PATH.set(dir.join("shell.log"));
}

pub fn log(msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{}.{:03}] {}", secs % 100000, {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_millis())
            .unwrap_or(0);
        ms
    }, msg);
    println!("{}", line);
    if let Some(p) = LOG_PATH.get() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{}", line);
        }
    }
}

/// Usage: slog!("engine ready on port {}", port);
#[macro_export]
macro_rules! slog {
    ($($arg:tt)*) => {
        $crate::logging::log(&format!($($arg)*))
    };
}
