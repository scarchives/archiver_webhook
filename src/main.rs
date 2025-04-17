use std::time::Duration;
use log::{info, warn, error};

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::init();
    info!("[scarchivebot] Starting up...");

    // TODO: Load config (from config.json or env)
    // TODO: Load users (from users.json)
    // TODO: Initialize ephemeral/in-memory DB
    // TODO: Initialize SoundCloud client (client ID fetch)
    // TODO: Start scheduler loop (every 60s)
    //   - For each user: fetch tracks, diff with DB, send new to Discord
    //   - Download/transcode audio (ffmpeg)
    //   - Send Discord webhook with embed + files
    // TODO: Clean up temp files

    // Example scheduler stub
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        // TODO: Poll SoundCloud for new tracks
        // TODO: Process and send to Discord
        info!("[scarchivebot] Tick");
    }
}
