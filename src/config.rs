use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use log::{info, warn, error};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    // Discord webhook URL for sending notifications
    pub discord_webhook_url: String,
    // Poll interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_sec: u64,
    // Path to the JSON file containing watchlisted user IDs
    #[serde(default = "default_users_file")]
    pub users_file: String,
    // Optional path to store database state (ephemeral if not specified)
    pub db_file: Option<String>,
    // Maximum tracks to fetch per user (prevents excessive API calls)
    #[serde(default = "default_max_tracks_per_user")]
    pub max_tracks_per_user: usize,
    // Temp directory for downloads (uses system temp if not specified)
    pub temp_dir: Option<String>,
}

fn default_poll_interval() -> u64 {
    60 // Default to 1 minute
}

fn default_users_file() -> String {
    "users.json".to_string()
}

fn default_max_tracks_per_user() -> usize {
    200
}

impl Default for Config {
    fn default() -> Self {
        Config {
            discord_webhook_url: "".to_string(),
            poll_interval_sec: default_poll_interval(),
            users_file: default_users_file(),
            db_file: None,
            max_tracks_per_user: default_max_tracks_per_user(),
            temp_dir: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Users {
    pub users: Vec<String>,
}

impl Config {
    pub fn load(config_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
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
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
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