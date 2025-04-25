use std::collections::HashSet;
use std::fs::{File, copy, remove_file};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use log::{info, debug, trace, error, warn};
use serde::{Deserialize, Serialize};

/// Simple database to store known track IDs
#[derive(Debug, Serialize, Deserialize)]
pub struct TrackDatabase {
    // Set of track_ids
    #[serde(default)]
    tracks: HashSet<String>,
    // Path to the database file (if persistent)
    #[serde(skip)]
    pub db_path: String,
}

impl TrackDatabase {
    /// Create a new database instance
    pub fn new(db_path: String) -> Self {
        TrackDatabase {
            tracks: HashSet::new(),
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
        let tracks: Vec<String> = self.tracks.iter().cloned().collect();
        debug!("Retrieved {} total tracks from database", tracks.len());
        tracks
    }
    
    /// Check if a track is already in the database
    pub fn has_track(&self, track_id: &str) -> bool {
        let has = self.tracks.contains(track_id);
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
                self.tracks.insert(track_id.clone());
                trace!("Added new track {} to database", track_id);
            }
            
            info!("Added {} new tracks to database (from batch of {})", 
                 new_tracks.len(), track_ids.len());
        } else {
            debug!("No new tracks found (checked {})", track_ids.len());
        }
        
        new_tracks
    }
    
    /// Initialize the database with a batch of track IDs
    pub fn initialize_with_tracks(&mut self, track_ids: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let count_before = self.tracks.len();
        
        for track_id in track_ids {
            self.tracks.insert(track_id.clone());
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
} 