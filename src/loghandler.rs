use std::fs::OpenOptions;
use std::io::Write;
use log::{LevelFilter, info, warn, error};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::time::Duration;
use tokio::time;

// Global stats
static TOTAL_TRACKS: AtomicU64 = AtomicU64::new(0);
static NEW_TRACKS: AtomicU64 = AtomicU64::new(0);
static ERROR_COUNT: AtomicU32 = AtomicU32::new(0);

/// Update the console title with current stats
pub fn update_console_title() {
    #[cfg(windows)]
    {
        use winapi::um::wincon::SetConsoleTitleW;
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        
        let title = format!(
            "SCArchive Webhook | Tracks: {} | New: {} | Errors: {}",
            TOTAL_TRACKS.load(Ordering::Relaxed),
            NEW_TRACKS.load(Ordering::Relaxed),
            ERROR_COUNT.load(Ordering::Relaxed)
        );
        
        let wide: Vec<u16> = OsStr::new(&title)
            .encode_wide()
            .chain(Some(0))
            .collect();
            
        unsafe {
            SetConsoleTitleW(wide.as_ptr());
        }
    }
}

/// Start the console title update task
pub fn start_console_title_updater() {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            update_console_title();
        }
    });
}

/// Increment the total tracks counter
pub fn increment_total_tracks(count: u64) {
    TOTAL_TRACKS.fetch_add(count, Ordering::Relaxed);
}

/// Increment the new tracks counter
pub fn increment_new_tracks(count: u64) {
    NEW_TRACKS.fetch_add(count, Ordering::Relaxed);
}

/// Increment the error counter
pub fn increment_error_count() {
    ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Setup logging to console and file
pub fn setup_logging(log_file: &str, log_level: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Configure the logger
    let level = match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => {
            warn!("Invalid log level '{}' in config, using 'info'", log_level);
            LevelFilter::Info
        }
    };
    
    // Initialize simple logger for console output
    simple_logger::SimpleLogger::new()
        .with_level(level)
        .env()
        .init()?;
    
    // Add a custom file logger hook (simple_logger doesn't support file output)
    let orig_logger = log::logger();
    let file_path = log_file.to_string();
    
    struct FileLogger {
        inner: Box<dyn log::Log>,
        file_path: String,
    }
    
    impl log::Log for FileLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            self.inner.enabled(metadata)
        }
        
        fn log(&self, record: &log::Record) {
            // First, let the original logger handle it
            self.inner.log(record);
            
            // Then write to file
            if self.enabled(record.metadata()) {
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.file_path) {
                        
                    let timestamp = chrono::Local::now()
                        .format("%Y-%m-%d %H:%M:%S%.3f");
                        
                    let log_line = format!(
                        "{} {} [{}] {}\n",
                        timestamp,
                        record.level(),
                        record.target(),
                        record.args()
                    );
                    
                    let _ = file.write_all(log_line.as_bytes());
                }
            }
        }
        
        fn flush(&self) {
            self.inner.flush();
        }
    }
    
    let logger = FileLogger {
        inner: Box::new(orig_logger),
        file_path,
    };
    
    log::set_boxed_logger(Box::new(logger))?;
    
    info!("Logging initialized: level={}, file={}", log_level, log_file);
    
    // Start the console title updater
    start_console_title_updater();
    
    Ok(())
} 