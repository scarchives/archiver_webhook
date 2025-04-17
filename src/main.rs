use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::env;
use log::{info, warn, error, debug};
use tokio::sync::Mutex;

mod audio;
mod config;
mod db;
mod discord;
mod soundcloud;

use config::{Config, Users};
use db::TrackDatabase;
use soundcloud::Track;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger
    env_logger::init();
    info!("[scarchivebot] Starting up...");

    // Check for command line args for URL resolution
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--resolve" && args.len() > 2 {
        return resolve_soundcloud_url(&args[2]).await;
    } else if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        println!("Usage:");
        println!("  scarchivebot                 - Run in watcher mode");
        println!("  scarchivebot --resolve URL   - Resolve a SoundCloud URL and display info");
        println!("  scarchivebot --help          - Show this help");
        return Ok(());
    }

    // Check for ffmpeg
    if !audio::check_ffmpeg() {
        warn!("ffmpeg not found in PATH, audio transcoding will not work!");
        warn!("Please install ffmpeg and make sure it's in your PATH");
    }

    // Run in watcher mode (default)
    run_watcher_mode().await
}

/// Resolve a SoundCloud URL and display information
async fn resolve_soundcloud_url(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("Resolving SoundCloud URL: {}", url);
    
    // Initialize SoundCloud client
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Resolve the URL
    let resolved = soundcloud::resolve_url(url).await?;
    
    // Check if this is a track
    if let Some(kind) = resolved.get("kind").and_then(|v| v.as_str()) {
        if kind == "track" {
            // Get the track ID
            if let Some(id) = resolved.get("id").and_then(|v| v.as_u64()) {
                let track_id = id.to_string();
                info!("URL resolved to track ID: {}", track_id);
                
                // Get detailed track info
                let track = soundcloud::get_track_details(&track_id).await?;
                
                // Print track details
                println!("\nTrack Information:");
                println!("Title: {}", track.title);
                println!("Artist: {}", track.user.username);
                
                if let Some(description) = &track.description {
                    if !description.is_empty() {
                        println!("Description: {}", description);
                    }
                }
                
                if let Some(count) = track.playback_count {
                    println!("Plays: {}", count);
                }
                
                if let Some(count) = track.likes_count {
                    println!("Likes: {}", count);
                }
                
                if let Some(genre) = &track.genre {
                    if !genre.is_empty() {
                        println!("Genre: {}", genre);
                    }
                }
                
                if let Some(tags) = &track.tag_list {
                    if !tags.is_empty() {
                        println!("Tags: {}", tags);
                    }
                }
                
                println!("Duration: {}:{:02}", track.duration / 1000 / 60, (track.duration / 1000) % 60);
                println!("URL: {}", track.permalink_url);
                
                if track.downloadable.unwrap_or(false) {
                    println!("Downloadable: Yes");
                } else {
                    println!("Downloadable: No");
                }
                
                return Ok(());
            }
        } else if kind == "user" {
            // Get the user ID
            if let Some(id) = resolved.get("id").and_then(|v| v.as_u64()) {
                let user_id = id.to_string();
                info!("URL resolved to user ID: {}", user_id);
                
                // Print user details
                println!("\nUser Information:");
                println!("Username: {}", resolved.get("username").and_then(|v| v.as_str()).unwrap_or("Unknown"));
                println!("ID: {}", user_id);
                println!("URL: {}", resolved.get("permalink_url").and_then(|v| v.as_str()).unwrap_or(""));
                
                if let Some(followers) = resolved.get("followers_count").and_then(|v| v.as_u64()) {
                    println!("Followers: {}", followers);
                }
                
                if let Some(tracks) = resolved.get("track_count").and_then(|v| v.as_u64()) {
                    println!("Tracks: {}", tracks);
                }
                
                // Print instructions for adding to watchlist
                println!("\nTo add this user to your watchlist, add the following ID to your users.json file:");
                println!("  {}", user_id);
                
                return Ok(());
            }
        }
    }
    
    // If we get here, something went wrong with the URL
    println!("URL resolved, but could not determine if it's a track or user.");
    println!("Raw data: {}", serde_json::to_string_pretty(&resolved)?);
    
    Ok(())
}

/// Run the bot in watcher mode (continuous monitoring)
async fn run_watcher_mode() -> Result<(), Box<dyn std::error::Error>> {
    // Load config
    let config_path = "config.json";
    let config = match Config::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };
    
    // Load users
    let users = match Users::load(&config.users_file) {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to load users from {}: {}", config.users_file, e);
            return Err(e);
        }
    };
    
    if users.users.is_empty() {
        warn!("No users found in {}. Add some users to the file and restart!", config.users_file);
    }
    
    // Initialize database
    let db_path = config.db_file.clone();
    let db = Arc::new(Mutex::new(match TrackDatabase::load_or_create(db_path) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to initialize database: {}", e);
            return Err(e);
        }
    }));
    
    // Initialize SoundCloud client
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Initialize users in database (if needed)
    {
        let mut db_guard = db.lock().await;
        for user_id in &users.users {
            db_guard.ensure_user(user_id);
        }
        // Save after initialization
        if let Err(e) = db_guard.save() {
            warn!("Failed to save database after initialization: {}", e);
        }
    }
    
    // Create scheduler interval
    let poll_interval = Duration::from_secs(config.poll_interval_sec);
    let mut interval = tokio::time::interval(poll_interval);
    
    // Start main polling loop
    info!("Starting polling loop with interval of {} seconds", config.poll_interval_sec);
    
    loop {
        // Wait for next tick
        interval.tick().await;
        info!("Polling for new tracks...");
        
        // For each user, check for new tracks
        for user_id in &users.users {
            match poll_user(&config, user_id, &db).await {
                Ok(new_count) => {
                    if new_count > 0 {
                        info!("Found {} new tracks for user {}", new_count, user_id);
                    } else {
                        debug!("No new tracks for user {}", user_id);
                    }
                }
                Err(e) => {
                    error!("Error polling user {}: {}", user_id, e);
                }
            }
        }
    }
}

/// Poll a user for new tracks, process them, and send to Discord
async fn poll_user(
    config: &Config,
    user_id: &str,
    db: &Arc<Mutex<TrackDatabase>>,
) -> Result<usize, Box<dyn std::error::Error>> {
    // Get existing track IDs for this user
    let known_tracks = {
        let db_guard = db.lock().await;
        db_guard.get_tracks(user_id)
    };
    
    debug!("User {} has {} known tracks", user_id, known_tracks.len());
    
    // Fetch latest tracks from SoundCloud
    let tracks = match soundcloud::get_user_tracks(user_id, config.max_tracks_per_user).await {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to fetch tracks for user {}: {}", user_id, e);
            return Err(e);
        }
    };
    
    debug!("Fetched {} tracks for user {}", tracks.len(), user_id);
    
    // Check which tracks are new
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    
    // Update database
    let new_track_ids = {
        let mut db_guard = db.lock().await;
        let new_ids = db_guard.add_tracks(user_id, &track_ids);
        
        // Save after update
        if !new_ids.is_empty() {
            if let Err(e) = db_guard.save() {
                warn!("Failed to save database after adding tracks: {}", e);
            }
        }
        
        new_ids
    };
    
    if new_track_ids.is_empty() {
        return Ok(0); // No new tracks
    }
    
    // Process new tracks (send webhook, etc.)
    let mut new_tracks_processed = 0;
    
    for track in tracks {
        // Check if this is a new track
        if !new_track_ids.contains(&track.id) {
            continue;
        }
        
        // Process the track
        info!("Processing new track: {} by {}", track.title, track.user.username);
        
        // Get detailed track info
        let track_details = match soundcloud::get_track_details(&track.id).await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to get details for track {}: {}", track.id, e);
                continue;
            }
        };
        
        // Download and transcode audio
        let audio_files = match audio::process_track_audio(&track_details, config.temp_dir.as_deref()).await {
            Ok((mp3, ogg)) => {
                let mut files = Vec::new();
                
                if let Some(path) = mp3 {
                    let filename = Path::new(&path)
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("track.mp3"))
                        .to_string_lossy()
                        .to_string();
                    files.push((path, filename));
                }
                
                if let Some(path) = ogg {
                    let filename = Path::new(&path)
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("track.ogg"))
                        .to_string_lossy()
                        .to_string();
                    files.push((path, filename));
                }
                
                files
            }
            Err(e) => {
                error!("Failed to process audio for track {}: {}", track.id, e);
                Vec::new()
            }
        };
        
        // Send to Discord
        match discord::send_track_webhook(&config.discord_webhook_url, &track_details, Some(audio_files.clone())).await {
            Ok(_) => {
                info!("Successfully sent webhook for track: {} by {}", 
                      track_details.title, track_details.user.username);
                new_tracks_processed += 1;
            }
            Err(e) => {
                error!("Failed to send webhook for track {}: {}", track.id, e);
            }
        }
        
        // Clean up temp files
        for (path, _) in audio_files {
            if let Err(e) = audio::delete_temp_file(&path).await {
                warn!("Failed to clean up temp file {}: {}", path, e);
            }
        }
    }
    
    Ok(new_tracks_processed)
}
