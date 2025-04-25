use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use log::{info, warn, debug, error};
use serde_json::Value;
use std::fs;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    // Discord webhook URL for sending notifications
    pub discord_webhook_url: String,
    /// Logging level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub log_level: String,
    // Poll interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_sec: u64,
    // Path to the JSON file containing watchlisted user IDs
    #[serde(default = "default_users_file")]
    pub users_file: String,
    // Path to the JSON file storing known track IDs
    #[serde(default = "default_tracks_file")]
    pub tracks_file: String,
    // Maximum tracks to fetch per user (prevents excessive API calls)
    #[serde(default = "default_max_tracks_per_user")]
    pub max_tracks_per_user: usize,
    // Number of tracks to fetch per API request (pagination size)
    #[serde(default = "default_pagination_size")]
    pub pagination_size: usize,
    // Buffer to add to user's track count to account for new uploads
    #[serde(default = "default_track_count_buffer")]
    pub track_count_buffer: usize,
    // Temp directory for downloads (uses system temp if not specified)
    pub temp_dir: Option<String>,
    /// Maximum number of parallel user fetches
    #[serde(default = "default_max_parallel_fetches")]
    pub max_parallel_fetches: usize,
    /// Maximum number of concurrent ffmpeg processes per user
    #[serde(default = "default_max_concurrent_processing")]
    pub max_concurrent_processing: usize,
    /// Whether to scrape and monitor user likes
    #[serde(default = "default_scrape_user_likes")]
    pub scrape_user_likes: bool,
    /// Maximum number of likes to fetch per user
    #[serde(default = "default_max_likes_per_user")]
    pub max_likes_per_user: usize,
    /// User ID or URL to monitor for new followings to add
    pub auto_follow_source: Option<String>,
    /// How often to check for new followings (in poll cycles)
    #[serde(default = "default_auto_follow_interval")]
    pub auto_follow_interval: usize,
    /// How often to save the database (in poll cycles)
    #[serde(default = "default_db_save_interval")]
    pub db_save_interval: usize,
}

fn default_poll_interval() -> u64 {
    60 // Default to 1 minute
}

fn default_users_file() -> String {
    "users.json".to_string()
}

fn default_tracks_file() -> String {
    "tracks.json".to_string()
}

fn default_max_tracks_per_user() -> usize {
    500 // Default to 500 total tracks per user (limit)
}

/// Default pagination size for SoundCloud API calls
fn default_pagination_size() -> usize {
    50 // Default to 50 tracks per API request
}

/// Default buffer to add to user's track count
fn default_track_count_buffer() -> usize {
    5 // Add 5 extra tracks to account for new uploads
}

/// Default log level if not specified in config.json
fn default_log_level() -> String {
    "info".to_string()
}

fn default_max_parallel_fetches() -> usize {
    4
}

/// Default maximum concurrent ffmpeg processes per user
fn default_max_concurrent_processing() -> usize {
    2
}

/// Default option for scraping user likes
fn default_scrape_user_likes() -> bool {
    false // Off by default to maintain backward compatibility
}

/// Default maximum number of likes to fetch per user
fn default_max_likes_per_user() -> usize {
    500 // Default to 500 likes per user (increased from 50)
}

/// Default interval for checking new follows (in poll cycles)
fn default_auto_follow_interval() -> usize {
    24 // Check once per day with default poll interval of 60 seconds
}

/// Default interval for saving the database (in poll cycles)
fn default_db_save_interval() -> usize {
    1 // Save after every poll by default
}

impl Default for Config {
    fn default() -> Self {
        Config {
            discord_webhook_url: "".to_string(),
            log_level: default_log_level(),
            poll_interval_sec: default_poll_interval(),
            users_file: default_users_file(),
            tracks_file: default_tracks_file(),
            max_tracks_per_user: default_max_tracks_per_user(),
            pagination_size: default_pagination_size(),
            track_count_buffer: default_track_count_buffer(),
            temp_dir: None,
            max_parallel_fetches: default_max_parallel_fetches(),
            max_concurrent_processing: default_max_concurrent_processing(),
            scrape_user_likes: default_scrape_user_likes(),
            max_likes_per_user: default_max_likes_per_user(),
            auto_follow_source: None,
            auto_follow_interval: default_auto_follow_interval(),
            db_save_interval: default_db_save_interval(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Users {
    pub users: Vec<String>,
}

impl Config {
    pub fn load(config_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if !Path::new(config_path).exists() {
            warn!("Config file not found at {}, creating default config", config_path);
            let default_config = Config::default();
            let json = serde_json::to_string_pretty(&default_config)?;
            std::fs::write(config_path, json)?;
            return Ok(default_config);
        }

        // Read the file as raw JSON Value first
        let file = File::open(config_path)?;
        let reader = BufReader::new(file);
        let config_json: Value = serde_json::from_reader(reader)?;
        
        // Start with the default config
        let mut config = Config::default();
        
        // Update only the fields that are present in the JSON
        if let Some(webhook_url) = config_json.get("discord_webhook_url").and_then(|v| v.as_str()) {
            config.discord_webhook_url = webhook_url.to_string();
        }
        
        if let Some(log_level) = config_json.get("log_level").and_then(|v| v.as_str()) {
            config.log_level = log_level.to_string();
        }
        
        if let Some(poll_interval) = config_json.get("poll_interval_sec").and_then(|v| v.as_u64()) {
            config.poll_interval_sec = poll_interval;
        }
        
        if let Some(users_file) = config_json.get("users_file").and_then(|v| v.as_str()) {
            config.users_file = users_file.to_string();
        }
        
        if let Some(tracks_file) = config_json.get("tracks_file").and_then(|v| v.as_str()) {
            config.tracks_file = tracks_file.to_string();
        }
        
        if let Some(max_tracks) = config_json.get("max_tracks_per_user").and_then(|v| v.as_u64()) {
            config.max_tracks_per_user = max_tracks as usize;
        }
        
        if let Some(pagination) = config_json.get("pagination_size").and_then(|v| v.as_u64()) {
            config.pagination_size = pagination as usize;
        }
        
        if let Some(buffer) = config_json.get("track_count_buffer").and_then(|v| v.as_u64()) {
            config.track_count_buffer = buffer as usize;
        }
        
        if let Some(temp_dir) = config_json.get("temp_dir") {
            if temp_dir.is_null() {
                config.temp_dir = None;
            } else if let Some(dir) = temp_dir.as_str() {
                config.temp_dir = Some(dir.to_string());
            }
        }
        
        if let Some(parallel) = config_json.get("max_parallel_fetches").and_then(|v| v.as_u64()) {
            config.max_parallel_fetches = parallel as usize;
        }
        
        if let Some(concurrent) = config_json.get("max_concurrent_processing").and_then(|v| v.as_u64()) {
            config.max_concurrent_processing = concurrent as usize;
        }
        
        if let Some(scrape_likes) = config_json.get("scrape_user_likes").and_then(|v| v.as_bool()) {
            config.scrape_user_likes = scrape_likes;
        }
        
        if let Some(max_likes) = config_json.get("max_likes_per_user").and_then(|v| v.as_u64()) {
            config.max_likes_per_user = max_likes as usize;
        }
        
        if let Some(auto_follow) = config_json.get("auto_follow_source") {
            if auto_follow.is_null() {
                config.auto_follow_source = None;
            } else if let Some(source) = auto_follow.as_str() {
                config.auto_follow_source = Some(source.to_string());
            }
        }
        
        if let Some(interval) = config_json.get("auto_follow_interval").and_then(|v| v.as_u64()) {
            config.auto_follow_interval = interval as usize;
        }
        
        if let Some(save_interval) = config_json.get("db_save_interval").and_then(|v| v.as_u64()) {
            config.db_save_interval = save_interval as usize;
        }
        
        // Validate required fields
        if config.discord_webhook_url.is_empty() {
            return Err("discord_webhook_url is required in config.json".into());
        }
        
        info!("Loaded configuration from {}", config_path);
        debug!("Config: log_level={}, poll_interval={}s, max_tracks={}, scrape_likes={}, max_concurrent_processing={}",
               config.log_level, config.poll_interval_sec, config.max_tracks_per_user, 
               config.scrape_user_likes, config.max_concurrent_processing);
        Ok(config)
    }
}

impl Users {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if !Path::new(path).exists() {
            warn!("Users file not found at {}, creating empty list", path);
            let empty_users = Users { users: Vec::new() };
            let json = serde_json::to_string_pretty(&empty_users)?;
            std::fs::write(path, json)?;
            return Ok(empty_users);
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let users: Users = serde_json::from_reader(reader)?;
        
        info!("Loaded {} users from {}", users.users.len(), path);
        Ok(users)
    }

    /// Save users list to a file
    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("Saving {} users to file: {}", self.users.len(), path);
        
        // First, create a backup of the existing file if it exists
        let backup_path = format!("{}.bak", path);
        if Path::new(path).exists() {
            debug!("Creating backup of existing users file");
            match fs::copy(path, &backup_path) {
                Ok(_) => debug!("Created backup at {}", backup_path),
                Err(e) => warn!("Failed to create backup file {}: {}", backup_path, e),
            }
        }
        
        // Write directly to target file
        let file = match File::create(path) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to create users file {}: {}", path, e);
                return Err(e.into());
            }
        };
        
        let writer = BufWriter::new(file);
        
        // Serialize to the file
        if let Err(e) = serde_json::to_writer_pretty(writer, self) {
            error!("Failed to write users to file: {}", e);
            
            // Try to restore from backup if it exists
            if Path::new(&backup_path).exists() {
                match fs::copy(&backup_path, path) {
                    Ok(_) => debug!("Restored from backup after write failure"),
                    Err(e2) => error!("Failed to restore from backup: {}", e2),
                }
            }
            
            return Err(e.into());
        }
        
        // Remove the backup file now that we've successfully written the new file
        if Path::new(&backup_path).exists() {
            if let Err(e) = fs::remove_file(&backup_path) {
                // This is not a critical error, just log a warning
                warn!("Failed to remove backup file {}: {}", backup_path, e);
            }
        }
        
        info!("Successfully saved {} users to {}", self.users.len(), path);
        Ok(())
    }
} 