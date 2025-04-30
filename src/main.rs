use std::sync::Arc;
use std::time::Duration;
use std::env;
use log::{info, warn, error, debug};
use tokio::sync::Mutex;
use crate::loghandler::{increment_new_tracks, increment_error_count, setup_logging};

mod audio;
mod config;
mod db;
mod discord;
mod soundcloud;
mod loghandler;
mod cli;

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
                return cli::resolve_soundcloud_url(&args[2]).await;
            },
            "--init-tracks" => {
                info!("Running in database initialization mode");
                return cli::initialize_tracks_database().await;
            },
            "--post-track" if args.len() > 2 => {
                info!("Running in post-track mode");
                return cli::post_single_track(&args[2]).await;
            },
            "--lookup-discord-id" if args.len() > 2 => {
                info!("Running in Discord ID lookup mode");
                return cli::lookup_by_discord_id(&args[2]).await;
            },
            "--generate-config" if args.len() > 2 => {
                info!("Running in config generation mode");
                return cli::generate_config(&args[2]).await;
            },
            "--help" | "-h" => {
                info!("Showing help information");
                cli::show_help();
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
    // Load config and initialize logging (console + file + console title updater)
    let config_path = "config.json";
    if let Ok(cfg) = Config::load(config_path) {
        if let Err(e) = setup_logging(&cfg.log_file, &cfg.log_level) {
            eprintln!("Failed to initialize logger: {}", e);
        }
    } else {
        // Fallback to defaults
        if let Err(e) = setup_logging("latest.log", "info") {
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
    
    // Initialize counters
    let mut total_polls = 0;
    let mut follow_check_counter = 0;
    let mut db_save_counter = 0;
    let mut tracks_since_last_save = 0;
    let mut db_needs_saving = false;

    // Main polling loop
    loop {
        total_polls += 1;
        info!("Starting poll #{}", total_polls);
        
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
        
        // Process users in batches with SoundCloud parallelism limit
        while users_processed < users_vec.len() {
            let batch_size = std::cmp::min(config.max_soundcloud_parallelism, users_vec.len() - users_processed);
            let batch = &users_vec[users_processed..users_processed + batch_size];
            
            let mut tasks = Vec::new();
            
            // Create tasks for each user in the batch
            for user_id in batch {
                let config = config.clone();
                let user_id = user_id.clone();
                let db = db.clone();
                
                let task = tokio::spawn(async move {
                    match poll_user(&config, &user_id, &db).await {
                        Ok(count) => {
                            increment_new_tracks(count as u64);
                            (user_id, Ok(count))
                        },
                        Err(e) => {
                            error!("Error polling user {}: {}", user_id, e);
                            increment_error_count();
                            (user_id, Err(e))
                        }
                    }
                });
                
                tasks.push(task);
            }
            
            // Wait for all tasks in the batch to complete
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
                        increment_error_count();
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
            
            info!("Saving database: {}", save_reason);
            
            // Hold the mutex lock for the entire save operation
            let db_guard = db.lock().await;
            if let Err(e) = db_guard.save() {
                error!("Failed to save tracks database: {}", e);
            } else {
                info!("Database saved successfully with {} tracks ({})", 
                     db_guard.get_all_tracks().len(), save_reason);
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
        
        // Sleep until next poll
        tokio::time::sleep(std::time::Duration::from_secs(config.poll_interval_sec)).await;
    }
    
    Ok(())
}

/// Poll a user for new tracks, process them, and send to Discord
async fn poll_user(
    config: &Config,
    user_id: &str,
    db: &Arc<Mutex<TrackDatabase>>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Create semaphores for limiting concurrency
    let processing_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_processing_parallelism));
    let discord_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_discord_parallelism));
    
    // Get mutable access to the database
    let mut db_guard = db.lock().await;
    
    // Use the poll_user method with both semaphores
    db_guard.poll_user(user_id, config, &processing_semaphore, &discord_semaphore).await
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
    
    // Use our new method to update followings
    users.update_followings_from_source(source, &config.users_file).await
}
