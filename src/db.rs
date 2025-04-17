use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use log::{info, warn, error};
use serde::{Deserialize, Serialize};

/// Database to store track IDs per user
#[derive(Debug, Serialize, Deserialize)]
pub struct TrackDatabase {
    // HashMap mapping user_id -> Vec of track_ids
    #[serde(default)]
    tracks: HashMap<String, Vec<String>>,
    // Path to the database file (if persistent)
    #[serde(skip)]
    db_path: Option<String>,
}

impl TrackDatabase {
    /// Create a new database instance
    pub fn new(db_path: Option<String>) -> Self {
        TrackDatabase {
            tracks: HashMap::new(),
            db_path,
        }
    }
    
    /// Load from file or create a new instance
    pub fn load_or_create(db_path: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        match &db_path {
            Some(path) => {
                if Path::new(path).exists() {
                    // Load database from file
                    let file = File::open(path)?;
                    let reader = BufReader::new(file);
                    let mut db: TrackDatabase = serde_json::from_reader(reader)?;
                    db.db_path = db_path.clone();
                    info!("Loaded database from {}", path);
                    Ok(db)
                } else {
                    // Create a new database and save it to file
                    let db = TrackDatabase::new(db_path.clone());
                    db.save()?;
                    info!("Created new database at {}", path);
                    Ok(db)
                }
            }
            None => {
                // In-memory database
                info!("Using ephemeral in-memory database");
                Ok(TrackDatabase::new(None))
            }
        }
    }
    
    /// Save database to file (if path is set)
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        match &self.db_path {
            Some(path) => {
                let file = File::create(path)?;
                let writer = BufWriter::new(file);
                serde_json::to_writer_pretty(writer, self)?;
                info!("Saved database to {}", path);
                Ok(())
            }
            None => {
                // No path set, just log and return success
                info!("Database is ephemeral, not saving to disk");
                Ok(())
            }
        }
    }
    
    /// Get all tracks for a user
    pub fn get_tracks(&self, user_id: &str) -> Vec<String> {
        match self.tracks.get(user_id) {
            Some(tracks) => tracks.clone(),
            None => Vec::new(),
        }
    }
    
    /// Check if a track is already in the database for a user
    pub fn has_track(&self, user_id: &str, track_id: &str) -> bool {
        match self.tracks.get(user_id) {
            Some(tracks) => tracks.contains(&track_id.to_string()),
            None => false,
        }
    }
    
    /// Add new tracks for a user and return which ones were newly added
    pub fn add_tracks(&mut self, user_id: &str, track_ids: &[String]) -> Vec<String> {
        let new_tracks: Vec<String> = track_ids
            .iter()
            .filter(|id| !self.has_track(user_id, id))
            .cloned()
            .collect();
            
        if !new_tracks.is_empty() {
            // Get or create the user's track list
            let user_tracks = self.tracks.entry(user_id.to_string()).or_insert_with(Vec::new);
            
            // Add the new tracks
            for track_id in &new_tracks {
                user_tracks.push(track_id.clone());
            }
            
            info!("Added {} new tracks for user {}", new_tracks.len(), user_id);
        }
        
        new_tracks
    }
    
    /// Initialize the DB for a user (if they don't exist yet)
    pub fn ensure_user(&mut self, user_id: &str) {
        if !self.tracks.contains_key(user_id) {
            self.tracks.insert(user_id.to_string(), Vec::new());
            info!("Initialized empty track list for user {}", user_id);
        }
    }
} 