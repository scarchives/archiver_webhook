use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::env;
use log::{info, warn, error, debug, trace};
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
    setup_logger();
    info!("[scarchivebot] Starting up v{}", env!("CARGO_PKG_VERSION"));
    
    // Log system info
    log_system_info();

    // Check for command line args
    let args: Vec<String> = env::args().collect();
    debug!("Command line arguments: {:?}", args);
    
    if args.len() > 1 {
        match args[1].as_str() {
            "--resolve" if args.len() > 2 => {
                info!("Running in URL resolution mode");
                return resolve_soundcloud_url(&args[2]).await;
            },
            "--init-tracks" => {
                info!("Running in database initialization mode");
                return initialize_tracks_database().await;
            },
            "--post-track" if args.len() > 2 => {
                info!("Running in post-track mode");
                return post_single_track(&args[2]).await;
            },
            "--help" | "-h" => {
                info!("Showing help information");
                println!("Usage:");
                println!("  scarchivebot                 - Run in watcher mode");
                println!("  scarchivebot --resolve URL   - Resolve a SoundCloud URL and display info");
                println!("  scarchivebot --init-tracks   - Initialize tracks database with existing tracks");
                println!("  scarchivebot --post-track ID - Post a specific track to webhook (bypass database)");
                println!("                               - Can be a track ID or a SoundCloud URL");
                println!("  scarchivebot --help          - Show this help");
                return Ok(());
            },
            _ => {
                warn!("Unknown command: {}", args[1]);
                println!("Unknown command. Use --help to see available commands.");
                return Ok(());
            }
        }
    }

    // Check for ffmpeg
    if !audio::check_ffmpeg() {
        warn!("ffmpeg not found in PATH, audio transcoding will not work!");
        warn!("Please install ffmpeg and make sure it's in your PATH");
    } else {
        info!("ffmpeg found in PATH");
    }

    // Run in watcher mode (default)
    info!("Running in watcher mode");
    run_watcher_mode().await
}

/// Setup logger with appropriate configuration
fn setup_logger() {
    // Get log level from environment or use default
    match env_logger::try_init() {
        Ok(_) => {
            // Successfully initialized logger
            let log_level = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
            debug!("Logger initialized with level: {}", log_level);
        },
        Err(e) => {
            // Print to stderr since logging isn't working
            eprintln!("Failed to initialize logger: {}", e);
        }
    }
}

/// Log system information
fn log_system_info() {
    debug!("System information:");
    debug!("  OS: {}", env::consts::OS);
    debug!("  Working directory: {:?}", env::current_dir().ok());
    debug!("  Executable: {:?}", env::current_exe().ok());
    debug!("  Temp directory: {:?}", env::temp_dir());
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
    info!("Fetching metadata from SoundCloud API");
    let resolved = match soundcloud::resolve_url(url).await {
        Ok(data) => {
            debug!("Successfully resolved URL");
            data
        },
        Err(e) => {
            error!("Failed to resolve URL: {}", e);
            return Err(e);
        }
    };
    
    // Check if this is a track
    if let Some(kind) = resolved.get("kind").and_then(|v| v.as_str()) {
        info!("Resolved object type: {}", kind);
        
        if kind == "track" {
            // Get the track ID
            if let Some(id) = resolved.get("id").and_then(|v| v.as_u64()) {
                let track_id = id.to_string();
                info!("URL resolved to track ID: {}", track_id);
                
                // Get detailed track info
                debug!("Fetching detailed track information");
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
                
                info!("Successfully displayed track information");
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
                
                info!("Successfully displayed user information");
                return Ok(());
            }
        }
    }
    
    // If we get here, something went wrong with the URL
    warn!("URL resolved, but could not determine if it's a track or user");
    println!("URL resolved, but could not determine if it's a track or user.");
    println!("Raw data: {}", serde_json::to_string_pretty(&resolved)?);
    
    Ok(())
}

/// Initialize tracks database with all existing tracks from all users
async fn initialize_tracks_database() -> Result<(), Box<dyn std::error::Error>> {
    // Load config
    let config_path = "config.json";
    info!("Loading configuration from {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => {
            debug!("Configuration loaded successfully");
            c
        },
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };
    
    // Load users
    info!("Loading users from {}", config.users_file);
    let users = match Users::load(&config.users_file) {
        Ok(u) => {
            info!("Loaded {} users", u.users.len());
            u
        },
        Err(e) => {
            error!("Failed to load users from {}: {}", config.users_file, e);
            return Err(e);
        }
    };
    
    if users.users.is_empty() {
        warn!("No users found in {}. Add some users to the file and restart!", config.users_file);
        return Err("No users found".into());
    }
    
    // Initialize database
    let tracks_db_path = config.tracks_file.clone();
    let mut db = match TrackDatabase::load_or_create(tracks_db_path) {
        Ok(d) => {
            info!("Tracks database initialized from {}", d.db_path);
            d
        },
        Err(e) => {
            error!("Failed to initialize tracks database: {}", e);
            return Err(e);
        }
    };
    
    // Initialize SoundCloud client
    info!("Initializing SoundCloud client");
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Track stats
    let mut total_users_processed = 0;
    let mut total_tracks_added = 0;
    
    // Process each user
    for user_id in &users.users {
        info!("Fetching tracks for user {}", user_id);
        match soundcloud::get_user_tracks(user_id, config.max_tracks_per_user).await {
            Ok(tracks) => {
                info!("Found {} tracks for user {}", tracks.len(), user_id);
                
                // Extract track IDs
                let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                
                // Add to database
                let current_count = db.get_all_tracks().len();
                db.initialize_with_tracks(&track_ids);
                let new_count = db.get_all_tracks().len();
                
                let added = new_count - current_count;
                total_tracks_added += added;
                
                info!("Added {} new tracks for user {} to database", added, user_id);
                total_users_processed += 1;
            },
            Err(e) => {
                error!("Failed to fetch tracks for user {}: {}", user_id, e);
            }
        }
    }
    
    // Save database
    if let Err(e) = db.save() {
        error!("Failed to save tracks database: {}", e);
        return Err(e);
    }
    
    println!("\nInitialization complete!");
    println!("Processed {} users", total_users_processed);
    println!("Added {} tracks to database", total_tracks_added);
    println!("Total tracks in database: {}", db.get_all_tracks().len());
    
    Ok(())
}

/// Run the bot in watcher mode (continuous monitoring)
async fn run_watcher_mode() -> Result<(), Box<dyn std::error::Error>> {
    // Load config
    let config_path = "config.json";
    info!("Loading configuration from {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => {
            debug!("Configuration loaded successfully");
            debug!("Poll interval: {} seconds", c.poll_interval_sec);
            debug!("Users file: {}", c.users_file);
            debug!("Tracks file: {}", c.tracks_file);
            debug!("Max tracks per user: {}", c.max_tracks_per_user);
            c
        },
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };
    
    // Load users
    info!("Loading users from {}", config.users_file);
    let users = match Users::load(&config.users_file) {
        Ok(u) => {
            info!("Loaded {} users", u.users.len());
            u
        },
        Err(e) => {
            error!("Failed to load users from {}: {}", config.users_file, e);
            return Err(e);
        }
    };
    
    if users.users.is_empty() {
        warn!("No users found in {}. Add some users to the file and restart!", config.users_file);
    } else {
        debug!("Loaded users: {:?}", users.users);
    }
    
    // Initialize database
    info!("Initializing tracks database");
    let tracks_db_path = config.tracks_file.clone();
    let db = Arc::new(Mutex::new(match TrackDatabase::load_or_create(tracks_db_path) {
        Ok(d) => {
            info!("Tracks database initialized from {} with {} tracks", 
                 d.db_path, d.get_all_tracks().len());
            d
        },
        Err(e) => {
            error!("Failed to initialize tracks database: {}", e);
            return Err(e);
        }
    }));
    
    // Initialize SoundCloud client
    info!("Initializing SoundCloud client");
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Create scheduler interval
    let poll_interval = Duration::from_secs(config.poll_interval_sec);
    let mut interval = tokio::time::interval(poll_interval);
    
    // Start main polling loop
    info!("Starting polling loop with interval of {} seconds", config.poll_interval_sec);
    
    // Track stats
    let mut total_polls = 0;
    let mut total_tracks_found = 0;
    let mut last_stats_time = std::time::Instant::now();
    
    loop {
        // Wait for next tick
        interval.tick().await;
        total_polls += 1;
        
        // Log periodic stats (every hour)
        let now = std::time::Instant::now();
        if now.duration_since(last_stats_time).as_secs() > 3600 {
            info!("Stats: {} polls completed, {} new tracks found", 
                 total_polls, total_tracks_found);
            last_stats_time = now;
        }
        
        debug!("Poll #{}: Checking for new tracks", total_polls);
        
        // For each user, check for new tracks
        for user_id in &users.users {
            trace!("Polling user {}", user_id);
            match poll_user(&config, user_id, &db).await {
                Ok(new_count) => {
                    total_tracks_found += new_count;
                    
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
        
        // Save the database after each poll
        {
            let mut db_guard = db.lock().await;
            if let Err(e) = db_guard.save() {
                warn!("Failed to save tracks database: {}", e);
            }
        }
        
        debug!("Poll #{} completed", total_polls);
    }
}

/// Poll a user for new tracks, process them, and send to Discord
async fn poll_user(
    config: &Config,
    user_id: &str,
    db: &Arc<Mutex<TrackDatabase>>,
) -> Result<usize, Box<dyn std::error::Error>> {
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
        db_guard.add_tracks(&track_ids)
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
        let processing_result = match audio::process_track_audio(&track_details, config.temp_dir.as_deref()).await {
            Ok((mp3, ogg, artwork, json)) => {
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
                
                if let Some(path) = artwork {
                    let filename = Path::new(&path)
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("cover.jpg"))
                        .to_string_lossy()
                        .to_string();
                    files.push((path, filename));
                }
                
                if let Some(path) = json {
                    let filename = Path::new(&path)
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("data.json"))
                        .to_string_lossy()
                        .to_string();
                    files.push((path, filename));
                }
                
                files
            },
            Err(e) => {
                error!("Failed to process audio for track {}: {}", track.id, e);
                Vec::new()
            }
        };
        
        // Send to Discord
        match discord::send_track_webhook(&config.discord_webhook_url, &track_details, Some(processing_result.clone())).await {
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
        for (path, _) in processing_result {
            if let Err(e) = audio::delete_temp_file(&path).await {
                warn!("Failed to clean up temp file {}: {}", path, e);
            }
        }
    }
    
    Ok(new_tracks_processed)
}

/// Post a single track to the webhook without checking the database
async fn post_single_track(id_or_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Load config
    let config_path = "config.json";
    info!("Loading configuration from {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => {
            debug!("Configuration loaded successfully");
            c
        },
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };
    
    // Initialize SoundCloud client
    info!("Initializing SoundCloud client");
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Check if this is a URL or an ID
    let track_id = if id_or_url.starts_with("http") {
        // This is a URL, resolve it
        info!("Resolving SoundCloud URL: {}", id_or_url);
        let resolved = match soundcloud::resolve_url(id_or_url).await {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to resolve URL: {}", e);
                return Err(e);
            }
        };
        
        // Check if it's a track
        if let Some(kind) = resolved.get("kind").and_then(|v| v.as_str()) {
            if kind == "track" {
                if let Some(id) = resolved.get("id").and_then(|v| v.as_u64()) {
                    let track_id = id.to_string();
                    info!("URL resolved to track ID: {}", track_id);
                    track_id
                } else {
                    error!("Could not extract track ID from resolved URL");
                    return Err("Could not extract track ID from resolved URL".into());
                }
            } else {
                error!("URL does not point to a track, but to a {}", kind);
                return Err(format!("URL points to a {}, not a track", kind).into());
            }
        } else {
            error!("Could not determine object type from resolved URL");
            return Err("Could not determine object type from resolved URL".into());
        }
    } else {
        // Assume this is a track ID
        id_or_url.to_string()
    };
    
    // Get track details
    info!("Fetching track details for ID: {}", track_id);
    let track_details = match soundcloud::get_track_details(&track_id).await {
        Ok(t) => {
            info!("Successfully fetched track: {} by {}", t.title, t.user.username);
            t
        },
        Err(e) => {
            error!("Failed to get track details: {}", e);
            return Err(e);
        }
    };
    
    // Download and transcode audio
    info!("Processing audio and artwork for track");
    let processing_result = match audio::process_track_audio(&track_details, config.temp_dir.as_deref()).await {
        Ok((mp3, ogg, artwork, json)) => {
            let mut files = Vec::new();
            
            if let Some(path) = mp3 {
                let file_path = path.clone();
                let filename = Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("track.mp3"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Generated MP3 file: {}", filename);
                files.push((file_path, filename));
            }
            
            if let Some(path) = ogg {
                let file_path = path.clone();
                let filename = Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("track.ogg"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Generated OGG file: {}", filename);
                files.push((file_path, filename));
            }
            
            if let Some(path) = artwork {
                let file_path = path.clone();
                let filename = Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("cover.jpg"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Downloaded artwork: {}", filename);
                files.push((file_path, filename));
            }
            
            if let Some(path) = json {
                let file_path = path.clone();
                let filename = Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("data.json"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Saved JSON metadata: {}", filename);
                files.push((file_path, filename));
            }
            
            files
        },
        Err(e) => {
            error!("Failed to process track media: {}", e);
            Vec::new() // Continue without audio files
        }
    };
    
    // Send to Discord
    info!("Sending webhook for track: {} by {}", track_details.title, track_details.user.username);
    match discord::send_track_webhook(&config.discord_webhook_url, &track_details, Some(processing_result.clone())).await {
        Ok(_) => {
            info!("Successfully sent webhook for track");
            println!("Track successfully posted to Discord: {} by {}", 
                   track_details.title, track_details.user.username);
        },
        Err(e) => {
            error!("Failed to send webhook: {}", e);
            return Err(e);
        }
    }
    
    // Clean up temp files
    for (path, _) in processing_result {
        if let Err(e) = audio::delete_temp_file(&path).await {
            warn!("Failed to clean up temp file {}: {}", path, e);
        }
    }
    
    Ok(())
}
