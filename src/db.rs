use std::collections::{HashSet, HashMap};
use std::fs::{File, copy, remove_file};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use log::{info, debug, trace, error, warn};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Discord message information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordMessage {
    /// Discord message ID
    pub id: String,
    /// Discord channel ID
    pub channel_id: Option<String>,
    /// User who originally posted the track
    pub user_id: Option<String>,
}

/// Simple database to store known track IDs
#[derive(Debug, Serialize, Deserialize)]
pub struct TrackDatabase {
    // Map of track_ids to Discord message info
    #[serde(default)]
    tracks: HashMap<String, Option<DiscordMessage>>,
    // Path to the database file (if persistent)
    #[serde(skip)]
    pub db_path: String,
}

impl TrackDatabase {
    /// Create a new database instance
    pub fn new(db_path: String) -> Self {
        TrackDatabase {
            tracks: HashMap::new(),
            db_path,
        }
    }
    
    /// Load from file or create a new instance
    pub fn load_or_create(db_path: String) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if Path::new(&db_path).exists() {
            // Load database from file
            debug!("Loading tracks database from {}", db_path);
            let file = File::open(&db_path)?;
            let reader = BufReader::new(file);
            let mut db: TrackDatabase = serde_json::from_reader(reader)?;
            db.db_path = db_path;
            
            let track_count = db.tracks.len();
            info!("Loaded tracks database with {} tracks", track_count);
            
            Ok(db)
        } else {
            // Create a new database and save it to file
            debug!("Tracks database file not found, creating new one at {}", db_path);
            let db = TrackDatabase::new(db_path);
            db.save()?;
            info!("Created new tracks database");
            Ok(db)
        }
    }
    
    /// Save database to file
    /// 
    /// Uses a safe file writing pattern to prevent data corruption
    /// in case of application crash or power loss during the save operation.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("Saving tracks database to {}", self.db_path);
        
        // Instead of creating a temp file and renaming it, we'll use a safer approach
        // that works better across platforms
        
        // First, create a backup of the existing file if it exists
        let backup_path = format!("{}.bak", self.db_path);
        if Path::new(&self.db_path).exists() {
            debug!("Creating backup of existing database file");
            match copy(&self.db_path, &backup_path) {
                Ok(_) => debug!("Created backup at {}", backup_path),
                Err(e) => warn!("Failed to create backup file {}: {}", backup_path, e),
            }
        }
        
        // Write directly to target file
        let file = match File::create(&self.db_path) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to create database file {}: {}", self.db_path, e);
                return Err(e.into());
            }
        };
        
        let writer = BufWriter::new(file);
        
        // Serialize to the file
        if let Err(e) = serde_json::to_writer_pretty(writer, self) {
            error!("Failed to write database to file: {}", e);
            
            // Try to restore from backup if it exists
            if Path::new(&backup_path).exists() {
                match copy(&backup_path, &self.db_path) {
                    Ok(_) => debug!("Restored from backup after write failure"),
                    Err(e2) => error!("Failed to restore from backup: {}", e2),
                }
            }
            
            return Err(e.into());
        }
        
        // Remove the backup file now that we've successfully written the new file
        if Path::new(&backup_path).exists() {
            if let Err(e) = remove_file(&backup_path) {
                // This is not a critical error, just log a warning
                warn!("Failed to remove backup file {}: {}", backup_path, e);
            }
        }
        
        let track_count = self.tracks.len();
        debug!("Tracks database saved with {} tracks", track_count);
        
        Ok(())
    }
    
    /// Get all tracks in the database
    pub fn get_all_tracks(&self) -> Vec<String> {
        let tracks: Vec<String> = self.tracks.keys().cloned().collect();
        debug!("Retrieved {} total tracks from database", tracks.len());
        tracks
    }
    
    /// Check if a track is already in the database
    pub fn has_track(&self, track_id: &str) -> bool {
        let has = self.tracks.contains_key(track_id);
        trace!("Track {} in database: {}", track_id, if has { "exists" } else { "new" });
        has
    }
    
    /// Add new tracks and return which ones were newly added
    /// 
    /// This method adds tracks to the in-memory database but does not automatically save to disk.
    /// To ensure persistence, call `save()` after adding tracks.
    pub fn add_tracks(&mut self, track_ids: &[String]) -> Vec<String> {
        debug!("Adding tracks to database: {} total to check", track_ids.len());
        
        let new_tracks: Vec<String> = track_ids
            .iter()
            .filter(|id| !self.has_track(id))
            .cloned()
            .collect();
            
        if !new_tracks.is_empty() {
            // Add the new tracks
            for track_id in &new_tracks {
                self.tracks.insert(track_id.clone(), None);
                trace!("Added new track {} to database", track_id);
            }
            
            info!("Added {} new tracks to database (from batch of {})", 
                 new_tracks.len(), track_ids.len());
        } else {
            debug!("No new tracks found (checked {})", track_ids.len());
        }
        
        new_tracks
    }
    
    /// Add a track with Discord message information
    pub fn add_track_with_discord_info(
        &mut self, 
        track_id: &str, 
        discord_id: String, 
        channel_id: Option<String>,
        user_id: Option<String>
    ) {
        let discord_info = DiscordMessage {
            id: discord_id,
            channel_id,
            user_id,
        };
        
        self.tracks.insert(track_id.to_string(), Some(discord_info));
        debug!("Added track {} with Discord message info", track_id);
    }
    
    /// Get Discord message info for a track if it exists
    pub fn get_discord_info(&self, track_id: &str) -> Option<DiscordMessage> {
        match self.tracks.get(track_id) {
            Some(Some(info)) => Some(info.clone()),
            _ => None,
        }
    }
    
    /// Initialize the database with a batch of track IDs
    pub fn initialize_with_tracks(&mut self, track_ids: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let count_before = self.tracks.len();
        
        for track_id in track_ids {
            self.tracks.insert(track_id.clone(), None);
        }
        
        let new_count = self.tracks.len() - count_before;
        info!("Initialized database with {} new tracks (total: {})", 
             new_count, self.tracks.len());
        
        // Save changes to disk
        self.save()?;
        
        Ok(())
    }
    
    /// Add tracks and immediately save to disk
    /// 
    /// This is a convenience method that adds tracks and then saves the database,
    /// ensuring that changes are persisted even if the application crashes.
    pub fn add_tracks_and_save(&mut self, track_ids: &[String]) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let new_tracks = self.add_tracks(track_ids);
        
        if !new_tracks.is_empty() {
            debug!("Saving database after adding {} new tracks", new_tracks.len());
            self.save()?;
        }
        
        Ok(new_tracks)
    }
    
    /// Perform a clean shutdown, ensuring all data is saved
    pub fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Performing clean database shutdown");
        self.save()?;
        info!("Database saved successfully with {} tracks", self.tracks.len());
        Ok(())
    }

    /// Initialize database with tracks from multiple users
    pub async fn initialize_with_tracks_from_users(
        &mut self, 
        users: &[String], 
        max_tracks_per_user: usize,
        pagination_size: usize,
        scrape_likes: bool,
        max_likes_per_user: usize,
    ) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
        let mut total_users_processed = 0;
        let mut total_tracks_added = 0;
        
        // Process each user
        for user_id in users {
            info!("Fetching tracks for user {}", user_id);
            
            // Collect all tracks from this user
            let mut all_tracks = Vec::new();
            
            // Get uploaded tracks
            match crate::soundcloud::get_user_tracks(user_id, max_tracks_per_user, pagination_size).await {
                Ok(tracks) => {
                    info!("Found {} uploaded tracks for user {}", tracks.len(), user_id);
                    all_tracks.extend(tracks);
                },
                Err(e) => {
                    error!("Failed to fetch tracks for user {}: {}", user_id, e);
                    continue;
                }
            }
            
            // If enabled, get liked tracks too
            if scrape_likes {
                info!("Fetching likes for user {} (enabled in config)", user_id);
                match crate::soundcloud::get_user_likes(user_id, max_likes_per_user, pagination_size).await {
                    Ok(likes) => {
                        let liked_tracks = crate::soundcloud::extract_tracks_from_likes(&likes);
                        info!("Found {} liked tracks for user {}", liked_tracks.len(), user_id);
                        all_tracks.extend(liked_tracks);
                    },
                    Err(e) => {
                        warn!("Failed to fetch likes for user {}: {}", user_id, e);
                    }
                }
            }
            
            // Extract track IDs
            let track_ids: Vec<String> = all_tracks.iter().map(|t| t.id.clone()).collect();
            info!("Total tracks for user {}: {}", user_id, track_ids.len());
            
            // Add to database
            let current_count = self.tracks.len();
            if let Err(e) = self.initialize_with_tracks(&track_ids) {
                error!("Failed to initialize database with tracks: {}", e);
                continue;
            }
            let new_count = self.tracks.len();
            
            let added = new_count - current_count;
            total_tracks_added += added;
            
            info!("Added {} new tracks for user {} to database", added, user_id);
            total_users_processed += 1;
        }
        
        Ok((total_users_processed, total_tracks_added))
    }

    /// Poll a user for new tracks and process them
    pub async fn poll_user(
        &mut self,
        user_id: &str,
        config: &crate::config::Config,
        processing_semaphore: &Arc<tokio::sync::Semaphore>
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        // Fetch latest tracks from SoundCloud
        let tracks = match crate::soundcloud::get_user_tracks(user_id, config.max_tracks_per_user, config.pagination_size).await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to fetch tracks for user {}: {}", user_id, e);
                return Err(e);
            }
        };
        
        debug!("Fetched {} tracks for user {}", tracks.len(), user_id);
        
        // If enabled, fetch user likes as well
        let mut all_tracks = tracks.clone();
        
        if config.scrape_user_likes {
            debug!("Fetching likes for user {} (enabled in config)", user_id);
            match crate::soundcloud::get_user_likes(user_id, config.max_likes_per_user, config.pagination_size).await {
                Ok(likes) => {
                    info!("Fetched {} likes for user {}", likes.len(), user_id);
                    
                    // Extract tracks from likes
                    let liked_tracks = crate::soundcloud::extract_tracks_from_likes(&likes);
                    debug!("Extracted {} tracks from user {}'s likes", liked_tracks.len(), user_id);
                    
                    // Add liked tracks to our collection
                    all_tracks.extend(liked_tracks);
                    debug!("Total tracks (uploads + likes): {}", all_tracks.len());
                },
                Err(e) => {
                    warn!("Failed to fetch likes for user {}: {}", user_id, e);
                    // Continue with just the user's tracks
                }
            }
        }
        
        // Check which tracks are new
        let track_ids: Vec<String> = all_tracks.iter().map(|t| t.id.clone()).collect();
        
        // Get new track IDs without adding to database yet
        let new_track_ids: Vec<String> = track_ids.iter()
            .filter(|id| !self.has_track(id))
            .cloned()
            .collect::<Vec<String>>();
        
        if new_track_ids.is_empty() {
            return Ok(0); // No new tracks
        }
        
        // Process new tracks in parallel with a resource limit for ffmpeg
        let mut tasks = Vec::new();
        let successful_tracks: Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>> = Arc::new(Mutex::new(Vec::new()));
        
        for track_id in &new_track_ids {
            // Find the track in our collection
            let track = match all_tracks.iter().find(|t| &t.id == track_id) {
                Some(t) => t.clone(),
                None => {
                    warn!("Could not find track {} in fetched tracks - skipping", track_id);
                    continue;
                }
            };
            
            let semaphore = Arc::clone(processing_semaphore);
            let successful_tracks = Arc::clone(&successful_tracks);
            
            // Spawn a task to process this track
            let webhook_url = config.discord_webhook_url.clone();
            let temp_dir = config.temp_dir.clone();
            let user_id_clone = user_id.to_string();
            let task = tokio::spawn(async move {
                // Acquire semaphore to limit concurrent ffmpeg processes
                let _permit = match semaphore.acquire().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        error!("Failed to acquire semaphore for track {}: {}", track.id, e);
                        return;
                    }
                };
                
                debug!("Processing new track: {} (ID: {})", track.title, track.id);
                
                // Get full track details
                let track_details = match crate::soundcloud::get_track_details(&track.id).await {
                    Ok(t) => t,
                    Err(e) => {
                        error!("Failed to get track details for {}: {}", track.id, e);
                        return;
                    }
                };
                
                // Download and process audio
                info!("Processing audio and artwork for track");
                let processing_result = match crate::audio::process_track_audio(&track_details, temp_dir.as_deref()).await {
                    Ok((audio_files, artwork, json)) => {
                        let mut files_for_discord = Vec::new();
                        
                        // Process all audio files
                        for (format_info, path) in &audio_files {
                            let file_path = path.clone();
                            let filename = std::path::Path::new(&file_path)
                                .file_name()
                                .unwrap_or_else(|| std::ffi::OsStr::new("track.audio"))
                                .to_string_lossy()
                                .to_string();
                            
                            info!("Audio file ({}): {}", format_info, filename);
                            files_for_discord.push((file_path, filename));
                        }
                        
                        // Add artwork if available
                        if let Some(path) = artwork {
                            let file_path = path.clone();
                            let filename = std::path::Path::new(&file_path)
                                .file_name()
                                .unwrap_or_else(|| std::ffi::OsStr::new("cover.jpg"))
                                .to_string_lossy()
                                .to_string();
                            
                            info!("Downloaded artwork: {}", filename);
                            files_for_discord.push((file_path, filename));
                        }
                        
                        // Add JSON metadata if available
                        if let Some(path) = json {
                            let file_path = path.clone();
                            let filename = std::path::Path::new(&file_path)
                                .file_name()
                                .unwrap_or_else(|| std::ffi::OsStr::new("data.json"))
                                .to_string_lossy()
                                .to_string();
                            
                            info!("Saved JSON metadata: {}", filename);
                            files_for_discord.push((file_path, filename));
                        }
                        
                        files_for_discord
                    },
                    Err(e) => {
                        error!("Failed to process audio for track {}: {}", track.id, e);
                        Vec::new()
                    }
                };
                
                // Send to Discord
                match crate::discord::send_track_webhook(&webhook_url, &track_details, Some(processing_result.clone())).await {
                    Ok(response) => {
                        info!("Successfully sent webhook for track: {} by {} (Discord message ID: {})", 
                              track_details.title, track_details.user.username, response.message_id);
                        let mut tracks = successful_tracks.lock().unwrap();
                        tracks.push((
                            track.id.clone(),
                            Some(response.message_id),
                            response.channel_id
                        ));
                    },
                    Err(e) => {
                        error!("Failed to send webhook for track {}: {}", track.id, e);
                    }
                };
                
                // Clean up temp files
                for (path, _) in processing_result.clone() {
                    if let Err(e) = crate::audio::delete_temp_file(&path).await {
                        warn!("Failed to clean up temp file {}: {}", path, e);
                    }
                }
            });
            
            tasks.push(task);
        }
        
        // Wait for all track processing tasks to complete
        let mut new_tracks_processed = 0;
        
        for task in tasks {
            match task.await {
                Ok(()) => {
                    new_tracks_processed += 1;
                },
                Err(e) => {
                    error!("Error in track processing task: {}", e);
                    crate::loghandler::increment_error_count();
                }
            }
        }
        
        // Add successful tracks to database with Discord info
        let successful_tracks_guard = successful_tracks.lock().unwrap();
        if !successful_tracks_guard.is_empty() {
            // Add successful tracks to the database with Discord message info
            for (track_id, message_id, channel_id) in successful_tracks_guard.iter() {
                if let Some(discord_id) = message_id {
                    // Add with Discord message info
                    self.add_track_with_discord_info(
                        track_id, 
                        discord_id.clone(), 
                        channel_id.clone(),
                        Some(user_id.to_string())
                    );
                } else {
                    // Just add the track without Discord info
                    self.tracks.insert(track_id.clone(), None);
                }
            }
            
            // Save the database
            if let Err(e) = self.save() {
                error!("Failed to save tracks database with Discord info: {}", e);
            } else {
                info!("Database saved with {} tracks including Discord message IDs", 
                     successful_tracks_guard.len());
                crate::loghandler::increment_total_tracks(successful_tracks_guard.len() as u64);
            }
        }
        
        Ok(new_tracks_processed)
    }
} 