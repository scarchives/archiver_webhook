use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use log::{info, warn, debug, error};
use serde_json::Value;
use std::fs;
use lazy_static;

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
    // Temp directory for downloads (uses system temp if not specified)
    pub temp_dir: Option<String>,
    /// Maximum number of parallel SoundCloud API requests (kept low to avoid rate limiting)
    #[serde(default = "default_max_soundcloud_parallelism")]
    pub max_soundcloud_parallelism: usize,
    /// Maximum number of parallel Discord webhook requests
    #[serde(default = "default_max_discord_parallelism")]
    pub max_discord_parallelism: usize,
    /// Maximum number of parallel processing tasks (ffmpeg, etc.)
    #[serde(default = "default_max_processing_parallelism")]
    pub max_processing_parallelism: usize,
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
    /// How many new tracks to process before saving the database
    #[serde(default = "default_db_save_tracks")]
    pub db_save_tracks: usize,
    /// Whether to show ffmpeg output in console
    #[serde(default = "default_show_ffmpeg_output")]
    pub show_ffmpeg_output: bool,
    /// Path to log file (defaults to latest.log)
    #[serde(default = "default_log_file")]
    pub log_file: String,
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

/// Default log level if not specified in config.json
fn default_log_level() -> String {
    "info".to_string()
}

/// Default value for max parallel SoundCloud API requests
fn default_max_soundcloud_parallelism() -> usize {
    2 // Default to 2 concurrent SoundCloud API requests to avoid rate limiting
}

/// Default value for max parallel Discord webhook requests
fn default_max_discord_parallelism() -> usize {
    4 // Default to 4 concurrent Discord webhook requests
}

/// Default value for max parallel processing tasks
fn default_max_processing_parallelism() -> usize {
    4 // Default to 4 concurrent processing tasks
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

/// Default number of tracks to process before saving the database
fn default_db_save_tracks() -> usize {
    50 // Save after processing 5 new tracks
}

/// Default setting for showing ffmpeg output
fn default_show_ffmpeg_output() -> bool {
    false // Off by default to reduce console clutter
}

/// Default log file path
fn default_log_file() -> String {
    "latest.log".to_string()
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
            temp_dir: None,
            max_soundcloud_parallelism: default_max_soundcloud_parallelism(),
            max_discord_parallelism: default_max_discord_parallelism(),
            max_processing_parallelism: default_max_processing_parallelism(),
            scrape_user_likes: default_scrape_user_likes(),
            max_likes_per_user: default_max_likes_per_user(),
            auto_follow_source: None,
            auto_follow_interval: default_auto_follow_interval(),
            db_save_interval: default_db_save_interval(),
            db_save_tracks: default_db_save_tracks(),
            show_ffmpeg_output: default_show_ffmpeg_output(),
            log_file: default_log_file(),
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
        
        if let Some(temp_dir) = config_json.get("temp_dir") {
            if temp_dir.is_null() {
                config.temp_dir = None;
            } else if let Some(dir) = temp_dir.as_str() {
                config.temp_dir = Some(dir.to_string());
            }
        }
        
        if let Some(soundcloud_parallelism) = config_json.get("max_soundcloud_parallelism").and_then(|v| v.as_u64()) {
            config.max_soundcloud_parallelism = soundcloud_parallelism as usize;
        }
        
        if let Some(discord_parallelism) = config_json.get("max_discord_parallelism").and_then(|v| v.as_u64()) {
            config.max_discord_parallelism = discord_parallelism as usize;
        }
        
        if let Some(processing_parallelism) = config_json.get("max_processing_parallelism").and_then(|v| v.as_u64()) {
            config.max_processing_parallelism = processing_parallelism as usize;
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
        
        if let Some(save_tracks) = config_json.get("db_save_tracks").and_then(|v| v.as_u64()) {
            config.db_save_tracks = save_tracks as usize;
        }
        
        if let Some(show_ffmpeg) = config_json.get("show_ffmpeg_output").and_then(|v| v.as_bool()) {
            config.show_ffmpeg_output = show_ffmpeg;
        }
        
        if let Some(log_file) = config_json.get("log_file").and_then(|v| v.as_str()) {
            config.log_file = log_file.to_string();
        }
        
        // Validate required fields
        if config.discord_webhook_url.is_empty() {
            return Err("discord_webhook_url is required in config.json".into());
        }
        
        info!("Loaded configuration from {}", config_path);
        debug!("Config: log_level={}, poll_interval={}s, max_tracks={}, scrape_likes={}, max_concurrent_processing={}",
               config.log_level, config.poll_interval_sec, config.max_tracks_per_user, 
               config.scrape_user_likes, config.max_processing_parallelism);
        Ok(config)
    }
    
    /// Static access to show_ffmpeg_output setting
    /// Used in audio.rs to check if ffmpeg output should be shown
    pub fn show_ffmpeg_output() -> Option<bool> {
        lazy_static::lazy_static! {
            static ref CONFIG_VALUE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);
        }
        
        let lock = CONFIG_VALUE.lock().unwrap();
        *lock
    }
    
    /// Set the value for the static show_ffmpeg_output access
    pub fn set_show_ffmpeg_output(value: bool) {
        lazy_static::lazy_static! {
            static ref CONFIG_VALUE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);
        }
        
        let mut lock = CONFIG_VALUE.lock().unwrap();
        *lock = Some(value);
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

    /// Update users list with new followings from a source user
    /// 
    /// This method fetches followings from a SoundCloud user and adds
    /// any new followings to the users list, then saves the changes.
    pub async fn update_followings_from_source(
        &mut self,
        source: &str,
        users_file: &str
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("Checking for new users followed by source: {}", source);
        
        // Initialize SoundCloud client if not already done
        if crate::soundcloud::get_client_id().is_none() {
            info!("Initializing SoundCloud client");
            match crate::soundcloud::initialize().await {
                Ok(_) => info!("SoundCloud client initialized successfully"),
                Err(e) => {
                    error!("Failed to initialize SoundCloud client: {}", e);
                    return Err(e);
                }
            }
        }
        
        // Determine if the source is an ID or URL
        let user_id = if source.contains("soundcloud.com") || source.contains("http") {
            // It's a URL, resolve it
            info!("Resolving URL to user ID: {}", source);
            match crate::soundcloud::resolve_url(source).await {
                Ok(data) => {
                    if let Some(kind) = data.get("kind").and_then(|v| v.as_str()) {
                        if kind == "user" {
                            match data.get("id").and_then(|v| v.as_u64()) {
                                Some(id) => id.to_string(),
                                None => {
                                    error!("Could not extract user ID from resolved URL data");
                                    return Err("Missing user ID in resolved data".into());
                                }
                            }
                        } else {
                            error!("URL resolved to non-user kind: {}", kind);
                            return Err(format!("URL resolved to non-user kind: {}", kind).into());
                        }
                    } else {
                        error!("URL resolved to object with missing kind");
                        return Err("URL resolved to object with missing kind".into());
                    }
                },
                Err(e) => {
                    error!("Failed to resolve URL {}: {}", source, e);
                    return Err(e);
                }
            }
        } else {
            // It's already an ID
            source.to_string()
        };
        
        // Fetch the user's followings
        info!("Fetching followings for user ID: {}", user_id);
        let followings = match crate::soundcloud::get_user_followings(&user_id, None).await {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to fetch followings: {}", e);
                return Err(e);
            }
        };
        
        info!("Found {} followings for source user", followings.len());
        
        // Extract user IDs from followings
        let following_ids: Vec<String> = followings.iter()
            .filter_map(|f| f.get("id").and_then(|v| v.as_u64()).map(|id| id.to_string()))
            .collect();
        
        // Find new followings not already in users list
        let new_followings: Vec<String> = following_ids.iter()
            .filter(|id| !self.users.contains(id))
            .cloned()
            .collect();
        
        let count = new_followings.len();
        
        if count > 0 {
            info!("Adding {} new followings to users list", count);
            for id in &new_followings {
                // Extract username if available for logging
                let username = followings.iter()
                    .find(|u| u.get("id").and_then(|v| v.as_u64()).map(|i| i.to_string()) == Some(id.clone()))
                    .and_then(|u| u.get("username").and_then(|v| v.as_str()))
                    .unwrap_or("Unknown");
                
                info!("Adding new user to watch: {} ({})", username, id);
                self.users.push(id.clone());
            }
            
            // Save updated users file
            match self.save(users_file) {
                Ok(_) => info!("Successfully saved {} new users to {}", count, users_file),
                Err(e) => {
                    error!("Failed to save updated users file: {}", e);
                    return Err(e);
                }
            }
        } else {
            debug!("No new followings found for user {}", user_id);
        }
        
        Ok(count)
    }
} 