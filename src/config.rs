use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use log::{info, warn};

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

        let file = File::open(config_path)?;
        let reader = BufReader::new(file);
        let config: Config = serde_json::from_reader(reader)?;
        
        // Validate required fields
        if config.discord_webhook_url.is_empty() {
            return Err("discord_webhook_url is required in config.json".into());
        }
        
        info!("Loaded configuration from {}", config_path);
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
} 