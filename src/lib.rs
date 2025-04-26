pub mod audio;
pub mod config;
pub mod db;
pub mod discord;
pub mod soundcloud;

// Re-export key structs for convenience
pub use config::{Config, Users};
pub use db::TrackDatabase;
pub use soundcloud::Track;

use std::fs::OpenOptions;
use std::io::Write;
use log::{LevelFilter, info, warn};

/// Initialize the application with the given config file
pub async fn initialize(config_path: &str) -> Result<(Config, Users, db::TrackDatabase), Box<dyn std::error::Error + Send + Sync>> {
    // Check for ffmpeg
    if !audio::check_ffmpeg() {
        log::warn!("ffmpeg not found in PATH, audio transcoding will not work!");
        log::warn!("Please install ffmpeg and make sure it's in your PATH");
    }

    // Load config
    let config = config::Config::load(config_path)?;
    
    // Setup logging
    setup_logging(&config.log_file, &config.log_level)?;
    
    // Set static ffmpeg output setting
    config::Config::set_show_ffmpeg_output(config.show_ffmpeg_output);
    
    // Load users
    let users = config::Users::load(&config.users_file)?;
    
    if users.users.is_empty() {
        log::warn!("No users found in {}. Add some users to the file and restart!", config.users_file);
    }
    
    // Initialize database
    let tracks_db_path = config.tracks_file.clone();
    let db = db::TrackDatabase::load_or_create(tracks_db_path)?;
    
    // Initialize SoundCloud client
    soundcloud::initialize().await?;
    
    Ok((config, users, db))
}

/// Setup logging to console and file
fn setup_logging(log_file: &str, log_level: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    Ok(())
} 