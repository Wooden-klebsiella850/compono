//! Logger vers fichier dans %APPDATA%\Compono\compono.log.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

struct FileLogger {
    file: Mutex<std::fs::File>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "[{}] [{}] {}", now(), record.level(), record.args());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

fn now() -> String {
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

/// Initialise le logger vers `dir/compono.log`.
pub fn init(dir: &Path) -> std::io::Result<()> {
    create_dir_all(dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("compono.log"))?;
    let logger = FileLogger {
        file: Mutex::new(file),
    };
    log::set_boxed_logger(Box::new(logger)).map_err(|err| std::io::Error::other(err.to_string()))?;
    log::set_max_level(LevelFilter::Debug);
    Ok(())
}
