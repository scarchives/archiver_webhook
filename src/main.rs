use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::env;
use std::io::{self, Write, BufRead};
use log::{info, warn, error, debug};
use tokio::sync::Mutex;
use simple_logger;
use log::LevelFilter;

mod audio;
mod config;
mod db;
mod discord;
mod soundcloud;

use config::{Config, Users};
use db::TrackDatabase;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logger
    setup_logger();
    info!("[scraper_webhook] Starting up v{}", env!("CARGO_PKG_VERSION"));
    
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
            "--generate-config" if args.len() > 2 => {
                info!("Running in config generation mode");
                return generate_config(&args[2]).await;
            },
            "--help" | "-h" => {
                info!("Showing help information");
                println!("Usage:");
                println!("  scraper_webhook                 - Run in watcher mode");
                println!("  scraper_webhook --resolve URL   - Resolve a SoundCloud URL and display info");
                println!("  scraper_webhook --init-tracks   - Initialize tracks database with existing tracks");
                println!("  scraper_webhook --post-track ID - Post a specific track to webhook (bypass database)");
                println!("                               - Can be a track ID or a SoundCloud URL");
                println!("  scraper_webhook --generate-config URL - Generate config.json and users.json files");
                println!("                               - URL should be a SoundCloud user profile");
                println!("  scraper_webhook --help          - Show this help");
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
    // Initialize simple logger with default info level
    if let Err(e) = simple_logger::init() {
        eprintln!("Failed to initialize logger: {}", e);
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
async fn resolve_soundcloud_url(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
async fn initialize_tracks_database() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
        match soundcloud::get_user_tracks(user_id, config.max_tracks_per_user, config.pagination_size, config.track_count_buffer).await {
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
async fn run_watcher_mode() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config_path = "config.json";
    info!("Loading configuration from {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => {
            // Configure log filter based on config
            match c.log_level.parse::<LevelFilter>() {
                Ok(level) => log::set_max_level(level),
                Err(_) => {
                    warn!("Invalid log_level '{}' in config.json; defaulting to 'info'", c.log_level);
                    log::set_max_level(LevelFilter::Info);
                }
            }
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
    
    // Log system info now that logger is configured
    log_system_info();

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
    let total_new_tracks_overall = 0;
    let mut last_stats_time = std::time::Instant::now();
    
    loop {
        // Wait for next tick
        interval.tick().await;
        total_polls += 1;
        
        // Log periodic stats (every hour)
        let now = std::time::Instant::now();
        if now.duration_since(last_stats_time).as_secs() > 3600 {
            info!("Stats: {} polls completed, {} new tracks found", 
                 total_polls, total_new_tracks_overall);
            last_stats_time = now;
        }
        
        debug!("Poll #{}: Checking for new tracks", total_polls);
        
        // Process users in parallel batches
        let users_vec = users.users.clone();
        let mut users_processed = 0;
        let mut total_new_tracks = 0;

        while users_processed < users_vec.len() {
            let batch_size = std::cmp::min(
                config.max_parallel_fetches,
                users_vec.len() - users_processed
            );
            
            let mut tasks = Vec::with_capacity(batch_size);
            
            // Create tasks for this batch
            for i in 0..batch_size {
                let user_id = users_vec[users_processed + i].clone();
                let config_clone = config.clone();
                let db_clone = db.clone();
                
                let task = tokio::spawn(async move {
                    match poll_user(&config_clone, &user_id, &db_clone).await {
                        Ok(new_count) => {
                            if new_count > 0 {
                                info!("Found {} new tracks for user {}", new_count, user_id);
                            } else {
                                debug!("No new tracks for user {}", user_id);
                            }
                            (user_id, Ok(new_count))
                        }
                        Err(e) => {
                            error!("Error polling user {}: {}", user_id, e);
                            (user_id, Err(e))
                        }
                    }
                });
                
                tasks.push(task);
            }
            
            // Wait for all tasks in this batch to complete
            for task in tasks {
                match task.await {
                    Ok((_user_id, Ok(count))) => total_new_tracks += count,
                    Ok((_user_id, Err(_))) => {
                        // Error already logged in poll_user
                    },
                    Err(e) => {
                        error!("Task join error: {}", e);
                    }
                }
            }
            
            users_processed += batch_size;
        }

        // Save the database after all users have been processed
        {
            let db_guard = db.lock().await;
            if let Err(e) = db_guard.save() {
                warn!("Failed to save tracks database: {}", e);
            }
        }

        if total_new_tracks > 0 {
            info!("Poll #{} completed: {} new tracks found", total_polls, total_new_tracks);
        } else {
            debug!("Poll #{} completed: no new tracks", total_polls);
        }
    }
}

/// Poll a user for new tracks, process them, and send to Discord
async fn poll_user(
    config: &Config,
    user_id: &str,
    db: &Arc<Mutex<TrackDatabase>>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Fetch latest tracks from SoundCloud
    let tracks = match soundcloud::get_user_tracks(user_id, config.max_tracks_per_user, config.pagination_size, config.track_count_buffer).await {
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
async fn post_single_track(id_or_url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

/// Generate config.json and users.json files interactively based on a SoundCloud user's followings
async fn generate_config(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Generating configuration based on SoundCloud user: {}", url);
    
    // Initialize SoundCloud client
    info!("Initializing SoundCloud client");
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Resolve the URL to get the user ID
    info!("Resolving SoundCloud URL: {}", url);
    let resolved = match soundcloud::resolve_url(url).await {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to resolve URL: {}", e);
            return Err(e);
        }
    };
    
    // Check if this is a user
    if let Some(kind) = resolved.get("kind").and_then(|v| v.as_str()) {
        if kind != "user" {
            error!("The URL does not point to a user account (found: {})", kind);
            return Err(format!("URL points to a {}, not a user", kind).into());
        }
    } else {
        error!("Could not determine object type from resolved URL");
        return Err("Could not determine object type from resolved URL".into());
    }
    
    // Get the user ID and username
    let user_id = match resolved.get("id").and_then(|v| v.as_u64()) {
        Some(id) => id.to_string(),
        None => {
            error!("Could not extract user ID from resolved URL");
            return Err("Could not extract user ID from resolved URL".into());
        }
    };
    
    let username = resolved.get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    
    info!("URL resolved to user ID: {} ({})", user_id, username);
    println!("\nFound user: {} (ID: {})", username, user_id);
    
    // Ask if the user should also be included in the users.json
    println!("\nDo you want to include {} in the users.json file? (Y/n): ", username);
    let include_user = read_line_with_default("y");
    let include_user = include_user.trim().to_lowercase() != "n";
    
    // Fetch the user's followings
    println!("\nFetching users that {} follows...", username);
    let followings = match soundcloud::get_user_followings(&user_id, None).await {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to fetch followings: {}", e);
            return Err(e);
        }
    };
    
    println!("Found {} users that {} follows.", followings.len(), username);
    
    // Extract user IDs and usernames
    let mut user_ids = Vec::new();
    if include_user {
        user_ids.push(user_id.clone());
    }
    
    // Calculate the maximum username length for formatting
    let max_username_len = followings.iter()
        .filter_map(|u| u.get("username").and_then(|v| v.as_str()).map(|s| s.len()))
        .max()
        .unwrap_or(10);
    
    // Display the users with track counts
    println!("\nFollowed users (with track counts):");
    println!("{:<5} {:<1} {:<width$} {:<10} {:<12}", 
             "No.", "", "Username", "Tracks", "ID", width=max_username_len);
    println!("{}", "-".repeat(5 + 1 + max_username_len + 10 + 12 + 3));
    
    for (i, user) in followings.iter().enumerate() {
        let following_id = user.get("id").and_then(|v| v.as_u64()).map(|id| id.to_string());
        let following_username = user.get("username").and_then(|v| v.as_str()).unwrap_or("Unknown");
        let track_count = user.get("track_count").and_then(|v| v.as_u64()).unwrap_or(0);
        
        if let Some(id) = &following_id {
            println!("{:<5} {:<1} {:<width$} {:<10} {:<12}", 
                     i+1, "", following_username, track_count, id, width=max_username_len);
            user_ids.push(id.clone());
        }
    }
    
    // Generate the config.json file
    println!("\nGenerating config.json and users.json files...");
    
    // Ask for config values
    println!("\nEnter Discord webhook URL [required]: ");
    let discord_webhook_url = read_line();
    if discord_webhook_url.trim().is_empty() {
        error!("Discord webhook URL is required");
        return Err("Discord webhook URL is required".into());
    }
    
    println!("\nEnter log level [info]: ");
    let log_level = read_line_with_default("info");
    
    println!("\nEnter poll interval in seconds [60]: ");
    let poll_interval_sec = read_line_with_default("60")
        .parse::<u64>()
        .unwrap_or(60);
    
    println!("\nEnter users file path [users.json]: ");
    let users_file = read_line_with_default("users.json");
    
    println!("\nEnter tracks file path [tracks.json]: ");
    let tracks_file = read_line_with_default("tracks.json");
    
    println!("\nEnter maximum tracks to fetch per user [500]: ");
    let max_tracks_per_user = read_line_with_default("500")
        .parse::<usize>()
        .unwrap_or(500);
    
    println!("\nEnter pagination size for API requests [50]: ");
    let pagination_size = read_line_with_default("50")
        .parse::<usize>()
        .unwrap_or(50);
    
    println!("\nEnter track count buffer [5]: ");
    let track_count_buffer = read_line_with_default("5")
        .parse::<usize>()
        .unwrap_or(5);
    
    println!("\nEnter temp directory [use system temp]: ");
    let temp_dir = read_line_with_default("");
    let temp_dir = if temp_dir.trim().is_empty() {
        None
    } else {
        Some(temp_dir)
    };
    
    println!("\nEnter maximum parallel user fetches [4]: ");
    let max_parallel_fetches = read_line_with_default("4")
        .parse::<usize>()
        .unwrap_or(4);
    
    // Create the config
    let config = Config {
        discord_webhook_url,
        log_level,
        poll_interval_sec,
        users_file: users_file.clone(),
        tracks_file,
        max_tracks_per_user,
        pagination_size,
        track_count_buffer,
        temp_dir,
        max_parallel_fetches,
    };
    
    // Create the users
    let users = Users {
        users: user_ids,
    };
    
    // Save config.json
    let config_json = serde_json::to_string_pretty(&config)?;
    std::fs::write("config.json", config_json)?;
    
    // Save users.json
    let users_json = serde_json::to_string_pretty(&users)?;
    std::fs::write(&users_file, users_json)?;
    
    println!("\nConfiguration completed!");
    println!("- Created config.json file");
    println!("- Created {} file with {} users", users_file, users.users.len());
    println!("\nYou can now run the application in watcher mode:\n  ./scraper_webhook");
    
    Ok(())
}

/// Read a line from stdin
fn read_line() -> String {
    let stdin = io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_ok() {
        line.trim().to_string()
    } else {
        String::new()
    }
}

/// Read a line from stdin with a default value
fn read_line_with_default(default: &str) -> String {
    let stdin = io::stdin();
    let mut line = String::new();
    
    if !default.is_empty() {
        print!("[{}]: ", default);
        io::stdout().flush().unwrap();
    }
    
    if stdin.lock().read_line(&mut line).is_ok() {
        let input = line.trim();
        if input.is_empty() {
            default.to_string()
        } else {
            input.to_string()
        }
    } else {
        default.to_string()
    }
}
