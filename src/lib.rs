pub mod audio;
pub mod config;
pub mod db;
pub mod discord;
pub mod soundcloud;
pub mod loghandler;

// Re-export key structs for convenience
pub use config::{Config, Users};
pub use db::TrackDatabase;
pub use soundcloud::Track;

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
    loghandler::setup_logging(&config.log_file, &config.log_level)?;
    
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