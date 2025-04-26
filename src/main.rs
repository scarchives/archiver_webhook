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
    info!("[archiver_webhook] Starting up v{}", env!("CARGO_PKG_VERSION"));
    
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
                println!("  archiver_webhook                 - Run in watcher mode");
                println!("  archiver_webhook --resolve URL   - Resolve a SoundCloud URL and display info");
                println!("  archiver_webhook --init-tracks   - Initialize tracks database with existing tracks");
                println!("  archiver_webhook --post-track ID - Post a specific track to webhook (bypass database)");
                println!("                               - Can be a track ID or a SoundCloud URL");
                println!("  archiver_webhook --generate-config URL - Generate config.json and users.json files");
                println!("                               - URL should be a SoundCloud user profile");
                println!("  archiver_webhook --help          - Show this help");
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
    // Load config to get log level if available
    let config_path = "config.json";
    let log_level = match std::fs::File::open(config_path) {
        Ok(file) => {
            let reader = std::io::BufReader::new(file);
            match serde_json::from_reader::<_, serde_json::Value>(reader) {
                Ok(config) => {
                    // Extract log level from config
                    config.get("log_level")
                        .and_then(|l| l.as_str())
                        .unwrap_or("info")
                        .to_string()
                },
                Err(_) => "info".to_string()
            }
        },
        Err(_) => "info".to_string()
    };
    
    // Parse log level string to LevelFilter
    let level_filter = match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };
    
    // Initialize logger with the configured level
    if let Err(e) = simple_logger::SimpleLogger::new()
        .with_level(level_filter)
        .init() {
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

/// Update the log level at runtime
fn update_log_level(level_str: &str) {
    let level_filter = match level_str.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => {
            warn!("Invalid log level '{}', defaulting to info", level_str);
            LevelFilter::Info
        }
    };
    
    // Set the log level
    log::set_max_level(level_filter);
    info!("Log level set to {}", level_str);
}

/// Resolve a SoundCloud URL and display information
async fn resolve_soundcloud_url(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Load config to get log level
    let config_path = "config.json";
    debug!("Loading configuration from {}", config_path);
    match Config::load(config_path) {
        Ok(c) => {
            debug!("Configuration loaded successfully");
            debug!("Log level: {}", c.log_level);
            // Update log level based on config
            update_log_level(&c.log_level);
            c
        },
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };
    
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
            debug!("Log level: {}", c.log_level);
            debug!("Users file: {}", c.users_file);
            debug!("Tracks file: {}", c.tracks_file);
            // Update log level based on config
            update_log_level(&c.log_level);
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
        
        // Collect all tracks from this user
        let mut all_tracks = Vec::new();
        
        // Get uploaded tracks
        match soundcloud::get_user_tracks(user_id, config.max_tracks_per_user, config.pagination_size, config.track_count_buffer).await {
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
        if config.scrape_user_likes {
            info!("Fetching likes for user {} (enabled in config)", user_id);
            match soundcloud::get_user_likes(user_id, config.max_likes_per_user, config.pagination_size).await {
                Ok(likes) => {
                    let liked_tracks = soundcloud::extract_tracks_from_likes(&likes);
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
        let current_count = db.get_all_tracks().len();
        if let Err(e) = db.initialize_with_tracks(&track_ids) {
            error!("Failed to initialize database with tracks: {}", e);
            continue;
        }
        let new_count = db.get_all_tracks().len();
        
        let added = new_count - current_count;
        total_tracks_added += added;
        
        info!("Added {} new tracks for user {} to database", added, user_id);
        total_users_processed += 1;
    }
    
    // Save database - this is now redundant but kept for safety
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
            // Log level is now set in setup_logger()
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
    let mut users = match Users::load(&config.users_file) {
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
    
    // If auto-follow is enabled, check for new followings on startup
    if config.auto_follow_source.is_some() {
        info!("Auto-follow is enabled, checking for new followings on startup");
        match update_followings_from_source(&config, &mut users).await {
            Ok(count) => {
                if count > 0 {
                    info!("Added {} new users to watch from auto-follow source during startup", count);
                } else {
                    info!("No new followings found from auto-follow source during startup");
                }
            },
            Err(e) => {
                warn!("Failed to update followings from source during startup: {}", e);
            }
        }
    }
    
    // Initialize signal handlers for clean shutdown
    #[cfg(unix)]
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("Failed to set up SIGINT handler");
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to set up SIGTERM handler");
    
    // Create scheduler interval
    let poll_interval = Duration::from_secs(config.poll_interval_sec);
    let mut interval = tokio::time::interval(poll_interval);
    
    // Start main polling loop
    info!("Starting polling loop with interval of {} seconds", config.poll_interval_sec);
    
    // Track stats
    let mut total_polls = 0;
    let total_new_tracks_overall = 0;
    let mut last_stats_time = std::time::Instant::now();
    
    // Counter for auto follow checking
    let mut follow_check_counter = 0;
    
    // Counter for database saving
    let mut db_save_counter = 0;
    let mut db_needs_saving = false;
    
    // Main loop with graceful shutdown support
    loop {
        // Wait for either the next tick or a shutdown signal
        #[cfg(unix)]
        let should_shutdown = tokio::select! {
            _ = interval.tick() => false,
            _ = sigint.recv() => {
                info!("Received SIGINT signal");
                true
            },
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal");
                true
            },
        };
        
        #[cfg(not(unix))]
        let should_shutdown = tokio::select! {
            _ = interval.tick() => false,
            result = tokio::signal::ctrl_c() => {
                match result {
                    Ok(()) => {
                        info!("Received Ctrl+C signal");
                        // Give some breathing room for signal handling
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        true
                    },
                    Err(e) => {
                        error!("Error handling Ctrl+C signal: {}", e);
                        true
                    }
                }
            },
        };
        
        if should_shutdown {
            info!("Shutdown signal received, performing clean shutdown");
            
            // Set a reasonable timeout for shutdown operations
            let shutdown_timeout = Duration::from_secs(5);
            
            // Create a timeout for the shutdown process
            let shutdown_result = tokio::time::timeout(shutdown_timeout, async {
                // Save the database
                {
                    let db_guard = db.lock().await;
                    if let Err(e) = db_guard.shutdown() {
                        error!("Error during database shutdown: {}", e);
                    }
                }
                
                // Small delay to ensure all resources are freed
                tokio::time::sleep(Duration::from_millis(100)).await;
            }).await;
            
            match shutdown_result {
                Ok(_) => info!("Application shutdown completed successfully"),
                Err(_) => warn!("Application shutdown timed out after {} seconds", shutdown_timeout.as_secs()),
            }
            
            break;
        }
        
        total_polls += 1;
        
        // Log periodic stats (every hour)
        let now = std::time::Instant::now();
        if now.duration_since(last_stats_time).as_secs() > 3600 {
            info!("Stats: {} polls completed, {} new tracks found", 
                 total_polls, total_new_tracks_overall);
            last_stats_time = now;
        }
        
        debug!("Poll #{}: Checking for new tracks", total_polls);
        
        // Check if it's time to update followings
        if config.auto_follow_source.is_some() {
            follow_check_counter += 1;
            
            if follow_check_counter >= config.auto_follow_interval {
                info!("Auto-follow interval reached ({} polls), checking for new followings", 
                      config.auto_follow_interval);
                
                match update_followings_from_source(&config, &mut users).await {
                    Ok(count) => {
                        if count > 0 {
                            info!("Added {} new users to watch from auto-follow source", count);
                        } else {
                            debug!("No new followings found from auto-follow source");
                        }
                    },
                    Err(e) => {
                        warn!("Failed to update followings from source: {}", e);
                    }
                }
                
                // Reset counter
                follow_check_counter = 0;
            }
        }
        
        // Process users in parallel batches
        let users_vec = users.users.clone();
        let mut users_processed = 0;
        let mut total_new_tracks = 0;
        
        // Track new tracks since last save for the per-track save feature
        let mut tracks_since_last_save = 0;

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
                    Ok((_user_id, Ok(count))) => {
                        total_new_tracks += count;
                        tracks_since_last_save += count;
                        if count > 0 {
                            db_needs_saving = true;
                        }
                    },
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

        // Increment the database save counter
        db_save_counter += 1;
        
        // Save the database if:
        // 1. We found new tracks and reached the track threshold OR
        // 2. It's time for a scheduled save based on poll cycles
        let save_by_tracks = db_needs_saving && tracks_since_last_save >= config.db_save_tracks;
        let save_by_interval = db_save_counter >= config.db_save_interval;
        
        if save_by_tracks || save_by_interval {
            let save_reason = if save_by_tracks {
                format!("processed {} new tracks (threshold: {})", 
                       tracks_since_last_save, config.db_save_tracks)
            } else {
                format!("reached poll interval {} (current: {})",
                       config.db_save_interval, db_save_counter)
            };
            
            debug!("Saving database: {}", save_reason);
            
            {
                let db_guard = db.lock().await;
                if let Err(e) = db_guard.save() {
                    warn!("Failed to save tracks database: {}", e);
                } else {
                    debug!("Database saved successfully ({})", save_reason);
                }
            }
            
            // Reset the counter and flag
            db_save_counter = 0;
            tracks_since_last_save = 0;
            db_needs_saving = false;
        }

        if total_new_tracks > 0 {
            info!("Poll #{} completed: {} new tracks found", total_polls, total_new_tracks);
        } else {
            debug!("Poll #{} completed: no new tracks", total_polls);
        }
    }
    
    Ok(())
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
    
    // If enabled, fetch user likes as well
    let mut all_tracks = tracks.clone();
    
    if config.scrape_user_likes {
        debug!("Fetching likes for user {} (enabled in config)", user_id);
        match soundcloud::get_user_likes(user_id, config.max_likes_per_user, config.pagination_size).await {
            Ok(likes) => {
                info!("Fetched {} likes for user {}", likes.len(), user_id);
                
                // Extract tracks from likes
                let liked_tracks = soundcloud::extract_tracks_from_likes(&likes);
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
    
    // Update database
    let new_track_ids = {
        let mut db_guard = db.lock().await;
        // Use the new add_tracks_and_save method to ensure immediate persistence
        match db_guard.add_tracks_and_save(&track_ids) {
            Ok(new_ids) => new_ids,
            Err(e) => {
                error!("Error adding and saving tracks: {}", e);
                return Err(e.into());
            }
        }
    };
    
    if new_track_ids.is_empty() {
        return Ok(0); // No new tracks
    }
    
    // Process new tracks in parallel with a resource limit for ffmpeg
    // Default to maximum of 2 concurrent ffmpeg processes per user task unless configured differently
    let max_concurrent_processing = config.max_concurrent_processing;
    let processing_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_processing));
    
    let mut tasks = Vec::new();
    
    for track_id in &new_track_ids {
        // Find the track in our collection
        let track = match all_tracks.iter().find(|t| &t.id == track_id) {
            Some(t) => t.clone(),
            None => {
                warn!("Could not find track {} in fetched tracks - skipping", track_id);
                continue;
            }
        };
        
        let semaphore_clone = processing_semaphore.clone();
        let webhook_url = config.discord_webhook_url.clone();
        let temp_dir = config.temp_dir.clone();
        
        // Spawn a task to process this track
        let task = tokio::spawn(async move {
            // Acquire semaphore to limit concurrent ffmpeg processes
            let _permit = match semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    error!("Failed to acquire semaphore for track {}: {}", track.id, e);
                    return false;
                }
            };
            
            debug!("Processing new track: {} (ID: {})", track.title, track.id);
            
            // Get full track details
            let track_details = match soundcloud::get_track_details(&track.id).await {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to get track details for {}: {}", track.id, e);
                    return false;
                }
            };
            
            // Download and process audio
            info!("Processing audio and artwork for track");
            let processing_result = match audio::process_track_audio(&track_details, temp_dir.as_deref()).await {
                Ok((audio_files, artwork, json)) => {
                    let mut files_for_discord = Vec::new();
                    
                    // Process all audio files
                    for (format_info, path) in &audio_files {
                        let file_path = path.clone();
                        let filename = Path::new(&file_path)
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
                        let filename = Path::new(&file_path)
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
                        let filename = Path::new(&file_path)
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
            match discord::send_track_webhook(&webhook_url, &track_details, Some(processing_result.clone())).await {
                Ok(_) => {
                    info!("Successfully sent webhook for track: {} by {}", 
                          track_details.title, track_details.user.username);
                },
                Err(e) => {
                    error!("Failed to send webhook for track {}: {}", track.id, e);
                }
            }
            
            // Clean up temp files
            for (path, _) in processing_result.clone() {
                if let Err(e) = audio::delete_temp_file(&path).await {
                    warn!("Failed to clean up temp file {}: {}", path, e);
                }
            }
            
            true // Indicate success
        });
        
        tasks.push(task);
    }
    
    // Wait for all track processing tasks to complete
    let mut new_tracks_processed = 0;
    
    for task in tasks {
        match task.await {
            Ok(success) => {
                if success {
                    new_tracks_processed += 1;
                }
            },
            Err(e) => {
                error!("Error in track processing task: {}", e);
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
            debug!("Log level: {}", c.log_level);
            debug!("Webhook URL: {}", c.discord_webhook_url);
            // Update log level based on config
            update_log_level(&c.log_level);
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
    
    // Download and process audio
    info!("Processing audio and artwork for track");
    let processing_result = match audio::process_track_audio(&track_details, config.temp_dir.as_deref()).await {
        Ok((audio_files, artwork, json)) => {
            let mut files = Vec::new();
            
            // Process all audio files
            for (format_info, path) in &audio_files {
                let file_path = path.clone();
                let filename = Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("track.audio"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Audio file ({}): {}", format_info, filename);
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
    for (path, _) in processing_result.clone() {
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
    
    println!("\nScrape user likes? (true/false) [false]: ");
    let scrape_user_likes = read_line_with_default("false")
        .parse::<bool>()
        .unwrap_or(false);
    
    println!("\nMaximum likes to fetch per user [500]: ");
    let max_likes_per_user = read_line_with_default("500")
        .parse::<usize>()
        .unwrap_or(500);
    
    println!("\nAdd a user ID or URL to auto-follow their followings? (leave empty to disable): ");
    let auto_follow_input = read_line_with_default("");
    let auto_follow_source = if auto_follow_input.trim().is_empty() {
        None
    } else {
        Some(auto_follow_input)
    };
    
    println!("\nHow often to check for new followings (in poll cycles) [24]: ");
    let auto_follow_interval = read_line_with_default("24")
        .parse::<usize>()
        .unwrap_or(24);
    
    println!("\nMaximum concurrent ffmpeg processes per user [2]: ");
    let max_concurrent_processing = read_line_with_default("2")
        .parse::<usize>()
        .unwrap_or(2);
    
    println!("\nHow often to save the database (in poll cycles) [1]: ");
    let db_save_interval = read_line_with_default("1")
        .parse::<usize>()
        .unwrap_or(1);

    println!("\nNumber of tracks to process before saving database [5]: ");
    let db_save_tracks = read_line_with_default("5")
        .parse::<usize>()
        .unwrap_or(5);
    
    println!("\nShow ffmpeg output in console? (true/false) [false]: ");
    let show_ffmpeg_output = read_line_with_default("false")
        .parse::<bool>()
        .unwrap_or(false);

    println!("\nEnter log file path [latest.log]: ");
    let log_file = read_line_with_default("latest.log");
    
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
        scrape_user_likes,
        max_likes_per_user,
        auto_follow_source,
        auto_follow_interval,
        max_concurrent_processing,
        db_save_interval,
        db_save_tracks,
        show_ffmpeg_output,
        log_file,
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
    println!("\nYou can now run the application in watcher mode:\n  ./archiver_webhook");
    
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

/// Check for new followings from a source user and add them to the watched users list
///
/// This function is used by the auto-follow feature, which automatically adds new users followed
/// by a source user to the watch list. It's called both on startup and periodically during the
/// application's run time according to the configured interval.
///
/// The function:
/// 1. Resolves the source URL to a user ID if needed
/// 2. Fetches all of the source user's followings
/// 3. Compares with existing users to find new followings
/// 4. Adds new followings to the watch list
/// 5. Saves the updated users file
///
/// If a user is unfollowed by the source, they remain in the users list.
async fn update_followings_from_source(
    config: &Config,
    users: &mut Users,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Return early if no auto-follow source is configured
    let source = match &config.auto_follow_source {
        Some(s) => s,
        None => {
            debug!("No auto-follow source configured, skipping followings update");
            return Ok(0);
        }
    };
    
    info!("Checking for new users followed by source: {}", source);
    
    // Initialize SoundCloud client if not already done
    if soundcloud::get_client_id().is_none() {
        info!("Initializing SoundCloud client");
        match soundcloud::initialize().await {
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
        match soundcloud::resolve_url(source).await {
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
        source.clone()
    };
    
    // Fetch the user's followings
    info!("Fetching followings for user ID: {}", user_id);
    let followings = match soundcloud::get_user_followings(&user_id, None).await {
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
        .filter(|id| !users.users.contains(id))
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
            users.users.push(id.clone());
        }
        
        // Save updated users file
        match users.save(&config.users_file) {
            Ok(_) => info!("Successfully saved {} new users to {}", count, config.users_file),
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
