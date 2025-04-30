use std::io::{self, Write, BufRead};
use log::{info, warn, error, debug};
use std::sync::Arc;

use crate::config::{Config, Users};
use crate::db::TrackDatabase;
use crate::soundcloud;
use crate::loghandler::update_log_level;

/// Display help information to the console
pub fn show_help() {
    println!("Usage:");
    println!("  archiver_webhook                 - Run in watcher mode");
    println!("  archiver_webhook --resolve URL   - Resolve a SoundCloud URL and display info");
    println!("  archiver_webhook --init-tracks   - Initialize tracks database with existing tracks");
    println!("  archiver_webhook --post-track ID - Post a specific track to webhook (bypass database)");
    println!("                               - Can be a track ID or a SoundCloud URL");
    println!("  archiver_webhook --lookup-discord-id ID - Look up a track by Discord message ID");
    println!("  archiver_webhook --generate-config URL - Generate config.json and users.json files");
    println!("                               - URL should be a SoundCloud user profile");
    println!("  archiver_webhook --help          - Show this help");
}

/// Resolve a SoundCloud URL and display information
pub async fn resolve_soundcloud_url(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    // Initialize SoundCloud client
    match soundcloud::initialize().await {
        Ok(_) => info!("SoundCloud client initialized successfully"),
        Err(e) => {
            error!("Failed to initialize SoundCloud client: {}", e);
            return Err(e);
        }
    }
    
    // Use the modularized function from soundcloud.rs
    soundcloud::display_soundcloud_info(url).await
}

/// Initialize tracks database with all existing tracks from all users
pub async fn initialize_tracks_database() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    // Use our new method to initialize the database with tracks from users
    info!("Initializing database with tracks from {} users", users.users.len());
    let (total_users_processed, total_tracks_added) = match db.initialize_with_tracks_from_users(
        &users.users,
        config.max_tracks_per_user,
        config.pagination_size,
        config.scrape_user_likes,
        config.max_likes_per_user
    ).await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to initialize database with tracks: {}", e);
            return Err(e);
        }
    };
    
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

/// Post a single track to the webhook without checking the database
pub async fn post_single_track(id_or_url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    // Initialize database to store the Discord message ID
    let tracks_db_path = config.tracks_file.clone();
    let mut db = match TrackDatabase::load_or_create(tracks_db_path) {
        Ok(d) => {
            debug!("Tracks database initialized from {}", d.db_path);
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
    
    // Create Discord semaphore
    let discord_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_discord_parallelism));
    
    // Use our modularized function to process and post the track
    let result = match soundcloud::process_and_post_track(
        id_or_url, 
        &config.discord_webhook_url, 
        config.temp_dir.as_deref(),
        Some(&discord_semaphore)
    ).await {
        Ok((track_id, user_id, webhook_response)) => {
            // Store the Discord message ID in the database
            db.add_track_with_discord_info(
                &track_id,
                webhook_response.message_id.clone(),
                webhook_response.channel_id.clone(),
                Some(user_id)
            );
            
            // Save the database
            if let Err(e) = db.save() {
                warn!("Failed to save track with Discord message ID to database: {}", e);
            } else {
                info!("Stored track {} with Discord message ID {} in database", 
                     track_id, webhook_response.message_id);
            }
            
            Ok(())
        },
        Err(e) => Err(e),
    };
    
    result
}

/// Generate config.json and users.json files interactively based on a SoundCloud user's followings
pub async fn generate_config(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    println!("\nEnter temp directory [use system temp]: ");
    let temp_dir = read_line_with_default("");
    let temp_dir = if temp_dir.trim().is_empty() {
        None
    } else {
        Some(temp_dir)
    };
    
    println!("\nEnter maximum parallel SoundCloud API requests [2]: ");
    println!("(Keep this low - 1 or 2 recommended to avoid rate limiting)");
    let max_soundcloud_parallelism = read_line_with_default("2")
        .parse::<usize>()
        .unwrap_or(2);
    
    println!("\nEnter maximum parallel Discord webhook requests [4]: ");
    let max_discord_parallelism = read_line_with_default("4")
        .parse::<usize>()
        .unwrap_or(4);
    
    println!("\nEnter maximum parallel processing tasks (ffmpeg, etc.) [4]: ");
    let max_processing_parallelism = read_line_with_default("4")
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
        temp_dir,
        max_soundcloud_parallelism,
        max_discord_parallelism,
        max_processing_parallelism,
        scrape_user_likes,
        max_likes_per_user,
        auto_follow_source,
        auto_follow_interval,
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
pub fn read_line() -> String {
    let stdin = io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_ok() {
        line.trim().to_string()
    } else {
        String::new()
    }
}

/// Read a line from stdin with a default value
pub fn read_line_with_default(default: &str) -> String {
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

/// Look up a track by its Discord message ID
pub async fn lookup_by_discord_id(discord_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config_path = "config.json";
    info!("Loading configuration from {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => {
            debug!("Configuration loaded successfully");
            debug!("Log level: {}", c.log_level);
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
    
    // Load database
    let tracks_db_path = config.tracks_file.clone();
    let db = match TrackDatabase::load_or_create(tracks_db_path) {
        Ok(d) => {
            debug!("Tracks database initialized from {}", d.db_path);
            d
        },
        Err(e) => {
            error!("Failed to initialize tracks database: {}", e);
            return Err(e);
        }
    };
    
    // Look up the track by Discord message ID
    if let Some(track_id) = db.find_track_by_discord_id(discord_id) {
        println!("\nFound track with Discord message ID {}:", discord_id);
        println!("- SoundCloud track ID: {}", track_id);
        
        // Get Discord info for additional details
        if let Some(discord_info) = db.get_discord_info(&track_id) {
            println!("- Discord message ID: {}", discord_info.id);
            if let Some(channel_id) = discord_info.channel_id {
                println!("- Discord channel ID: {}", channel_id);
            }
            if let Some(user_id) = discord_info.user_id {
                println!("- Posted by user ID: {}", user_id);
            }
        }
        
        // Initialize SoundCloud client to get track details
        info!("Initializing SoundCloud client to get track details");
        match soundcloud::initialize().await {
            Ok(_) => info!("SoundCloud client initialized successfully"),
            Err(e) => {
                error!("Failed to initialize SoundCloud client: {}", e);
                return Err(e);
            }
        }
        
        // Get track details
        match soundcloud::get_track_details(&track_id).await {
            Ok(track) => {
                println!("\nTrack details:");
                println!("- Title: {}", track.title);
                println!("- Artist: {}", track.user.username);
                println!("- URL: {}", track.permalink_url);
                
                if let Some(desc) = &track.description {
                    if !desc.is_empty() {
                        println!("- Description: {}", desc);
                    }
                }
                
                if let Some(genre) = &track.genre {
                    if !genre.is_empty() {
                        println!("- Genre: {}", genre);
                    }
                }
                
                let duration_mins = track.duration / 1000 / 60;
                let duration_secs = (track.duration / 1000) % 60;
                println!("- Duration: {}:{:02}", duration_mins, duration_secs);
                
                if let Some(plays) = track.playback_count {
                    println!("- Plays: {}", plays);
                }
                
                if let Some(likes) = track.likes_count {
                    println!("- Likes: {}", likes);
                }
                
                if track.downloadable.unwrap_or(false) {
                    println!("- Downloadable: Yes");
                }
            },
            Err(e) => {
                println!("\nFailed to fetch track details: {}", e);
                println!("The track might have been deleted from SoundCloud.");
            }
        }
        
        Ok(())
    } else {
        println!("No track found with Discord message ID: {}", discord_id);
        Ok(())
    }
} 