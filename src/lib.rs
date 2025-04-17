pub mod audio;
pub mod config;
pub mod db;
pub mod discord;
pub mod soundcloud;

// Re-export key structs for convenience
pub use config::{Config, Users};
pub use db::TrackDatabase;
pub use soundcloud::Track;

/// Initialize the application with the given config file
pub async fn initialize(config_path: &str) -> Result<(Config, Users, db::TrackDatabase), Box<dyn std::error::Error>> {
    // Check for ffmpeg
    if !audio::check_ffmpeg() {
        log::warn!("ffmpeg not found in PATH, audio transcoding will not work!");
        log::warn!("Please install ffmpeg and make sure it's in your PATH");
    }

    // Load config
    let config = config::Config::load(config_path)?;
    
    // Load users
    let users = config::Users::load(&config.users_file)?;
    
    if users.users.is_empty() {
        log::warn!("No users found in {}. Add some users to the file and restart!", config.users_file);
    }
    
    // Initialize database
    let db_path = config.db_file.clone();
    let mut db = db::TrackDatabase::load_or_create(db_path)?;
    
    // Initialize SoundCloud client
    soundcloud::initialize().await?;
    
    // Initialize users in database (if needed)
    for user_id in &users.users {
        db.ensure_user(user_id);
    }
    
    // Save after initialization
    if let Err(e) = db.save() {
        log::warn!("Failed to save database after initialization: {}", e);
    }
    
    Ok((config, users, db))
} 