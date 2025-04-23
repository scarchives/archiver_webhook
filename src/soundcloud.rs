use std::sync::Mutex;
use std::time::Duration;
use log::{info, warn, error, debug};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;

// Global client ID cache
lazy_static::lazy_static! {
    static ref CLIENT_ID: Mutex<Option<String>> = Mutex::new(None);
    static ref HTTP_CLIENT: Client = Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .build()
        .unwrap();
    static ref SCRIPT_REGEX: Regex = Regex::new(r#"<script crossorigin src="(https://a-v2\.sndcdn\.com/assets/[^"]+)"></script>"#).unwrap();
    static ref CLIENT_ID_REGEX: Regex = Regex::new(r#"client_id:"([^"]+)"#).unwrap();
}

/// Track metadata returned from the SoundCloud API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub permalink_url: String,
    pub artwork_url: Option<String>,
    pub description: Option<String>,
    pub user: TrackUser,
    pub created_at: String,
    pub duration: u64,
    // Stream URLs
    pub stream_url: Option<String>,
    pub hls_url: Option<String>,
    pub download_url: Option<String>,
    // Stats
    pub playback_count: Option<u64>,
    pub likes_count: Option<u64>,
    pub reposts_count: Option<u64>,
    pub comment_count: Option<u64>,
    // Additional metadata
    pub genre: Option<String>,
    pub tag_list: Option<String>,
    pub downloadable: Option<bool>,
    // Raw JSON data
    #[serde(skip)]
    pub raw_data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackUser {
    pub id: String,
    pub username: String,
    pub permalink_url: String,
    pub avatar_url: Option<String>,
}

/// Initialize the SoundCloud client
pub async fn initialize() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Generate the initial client ID
    info!("Initializing SoundCloud client...");
    let initial_id = generate_client_id().await?;
    
    // Store it in the global cache
    let mut client_id = CLIENT_ID.lock().unwrap();
    *client_id = Some(initial_id.clone());
    
    info!("Generated initial SoundCloud client ID: {}", initial_id);
    Ok(())
}

/// Get the current SoundCloud client ID
pub fn get_client_id() -> Option<String> {
    let client_id = CLIENT_ID.lock().unwrap();
    client_id.clone()
}

/// Refresh the SoundCloud client ID
pub async fn refresh_client_id() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let new_id = generate_client_id().await?;
    
    // Update the global cache
    {
        let old_id = get_client_id();
        let mut client_id = CLIENT_ID.lock().unwrap();
        *client_id = Some(new_id.clone());
        
        if let Some(old) = old_id {
            info!("Refreshed SoundCloud client ID: {} -> {}", old, new_id);
        } else {
            info!("Set initial SoundCloud client ID: {}", new_id);
        }
    }
    
    Ok(new_id)
}

/// Generate a new SoundCloud client ID by scraping the website
async fn generate_client_id() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    
    debug!("Fetching SoundCloud homepage to extract client ID...");
    // Fetch the SoundCloud homepage
    let html = client
        .get("https://soundcloud.com")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await?
        .text()
        .await?;
    
    // Extract script URLs
    let matches: Vec<_> = SCRIPT_REGEX.captures_iter(&html).collect();
    
    if matches.is_empty() {
        error!("No script URLs found on SoundCloud homepage - site structure may have changed");
        return Err("No script URLs found on SoundCloud homepage".into());
    }
    
    debug!("Found {} potential script URLs to check for client ID", matches.len());
    
    // Check each script for the client ID
    
    for (idx, cap) in matches.iter().enumerate() {
        if let Some(script_url) = cap.get(1) {
            debug!("Checking script {}/{}: {}", idx + 1, matches.len(), script_url.as_str());
            let script_content = match client
                .get(script_url.as_str())
                .send()
                .await
            {
                Ok(res) => {
                    if !res.status().is_success() {
                        warn!("Script fetch returned status {}: {}", res.status(), script_url.as_str());
                        continue;
                    }
                    res.text().await?
                },
                Err(e) => {
                    warn!("Failed to fetch script {}: {}", script_url.as_str(), e);
                    continue;
                }
            };
            
            if let Some(client_id_match) = CLIENT_ID_REGEX.captures(&script_content) {
                if let Some(client_id) = client_id_match.get(1) {
                    let id = client_id.as_str().to_string();
                    debug!("Successfully extracted client ID from script {}: {}", idx + 1, id);
                    return Ok(id);
                }
            }
        }
    }
    
    error!("Could not find client_id in any SoundCloud scripts - site structure may have changed");
    Err("Could not find client_id in any script".into())
}

/// Get tracks for a SoundCloud user
pub async fn get_user_tracks(
    user_id: &str, 
    limit: usize,
    pagination_size: usize,
    track_count_buffer: usize
) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    let mut tracks = Vec::new();
    let mut offset = 0;
    
    // Use pagination_size for API requests, but cap it at 50 (API limit)
    let chunk_size = std::cmp::min(50, pagination_size);
    
    // First, try to get the user's details to check for total track count
    let effective_limit = match get_user_details(user_id).await {
        Ok(user_data) => {
            if let Some(track_count) = user_data.get("track_count").and_then(|v| v.as_u64()) {
                let track_count = track_count as usize;
                info!("User {} has {} tracks according to their profile", user_id, track_count);
                
                // Add buffer to account for potential new tracks since profile was loaded
                let buffered_count = track_count + track_count_buffer;
                
                // Use the smaller of the configured limit or actual track count (with buffer)
                let effective_limit = std::cmp::min(limit, buffered_count);
                if effective_limit < limit {
                    info!("Will fetch up to {} tracks (user has {} tracks + {} buffer)", 
                        effective_limit, track_count, track_count_buffer);
                }
                effective_limit
            } else {
                info!("Could not determine track count for user {}, using configured limit", user_id);
                limit
            }
        },
        Err(e) => {
            warn!("Failed to get user details for {}: {}. Using configured limit.", user_id, e);
            limit
        }
    };
    
    info!("Fetching up to {} tracks for user {} (with pagination size: {})", 
          effective_limit, user_id, chunk_size);
    
    // Get the current client ID or refresh it
    let mut client_id = match get_client_id() {
        Some(id) => {
            debug!("Using cached client ID: {}", id);
            id
        },
        None => {
            debug!("No cached client ID, generating new one");
            refresh_client_id().await?
        },
    };
    
    while tracks.len() < effective_limit {
        let current_limit = std::cmp::min(chunk_size, effective_limit - tracks.len());
        let url = format!(
            "https://api-v2.soundcloud.com/users/{}/tracks?client_id={}&offset={}&limit={}",
            user_id, client_id, offset, current_limit
        );
        
        debug!("Fetching tracks batch: offset={}, limit={}", offset, current_limit);
        
        // Make the request with retry logic
        let mut response_json = None;
        let max_retries = 3;
        
        for retry in 0..max_retries {
            if retry > 0 {
                debug!("Retrying tracks fetch (attempt {}/{}) for user {}", 
                      retry + 1, max_retries, user_id);
                sleep(Duration::from_secs(2 * retry as u64)).await;
            }
            
            let response = match client.get(&url).send().await {
                Ok(res) => {
                    if !res.status().is_success() {
                        // Check for auth error and refresh client ID
                        if res.status().as_u16() == 401 || res.status().as_u16() == 403 {
                            warn!("Auth error ({}), refreshing client ID", res.status());
                            client_id = refresh_client_id().await?;
                            continue;
                        }
                        
                        warn!("API error: HTTP {} when fetching tracks for user {}", res.status(), user_id);
                        continue;
                    }
                    res
                }
                Err(e) => {
                    warn!("Network error when fetching tracks for user {}: {}", user_id, e);
                    continue;
                }
            };
            
            match response.json::<Value>().await {
                Ok(json) => {
                    response_json = Some(json);
                    break;
                }
                Err(e) => {
                    warn!("JSON parse error for tracks response: {}", e);
                    if retry == max_retries - 1 {
                        return Err(format!("Failed to parse JSON after {} retries", max_retries).into());
                    }
                }
            }
        }
        
        let json = match response_json {
            Some(j) => j,
            None => {
                error!("Failed to fetch tracks for user {} after {} retries", user_id, max_retries);
                return Err(format!("Failed to fetch tracks for user {} after {} retries", 
                                  user_id, max_retries).into());
            }
        };
        
        // Extract the collection of tracks
        let collection = match json.get("collection") {
            Some(Value::Array(arr)) => arr,
            _ => {
                error!("Unexpected API response format for user {}: missing 'collection' array", user_id);
                return Err(format!("Unexpected API response format for user {}", user_id).into());
            }
        };
        
        if collection.is_empty() {
            debug!("No more tracks found for user {} at offset {}", user_id, offset);
            break; // No more tracks
        }
        
        debug!("Processing {} tracks from response", collection.len());
        
        // Parse the tracks
        let mut batch_count = 0;
        for track_json in collection {
            // Extract basic fields
            if let Some(id) = track_json.get("id").and_then(Value::as_u64) {
                let title = track_json.get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled")
                    .to_string();
                
                debug!("Processing track: {} (ID: {})", title, id);
                
                let track = Track {
                    id: id.to_string(),
                    title,
                    permalink_url: track_json.get("permalink_url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    artwork_url: track_json.get("artwork_url")
                        .and_then(Value::as_str)
                        .map(String::from),
                    description: track_json.get("description")
                        .and_then(Value::as_str)
                        .map(String::from),
                    user: parse_track_user(track_json),
                    created_at: track_json.get("created_at")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    duration: track_json.get("duration")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    stream_url: track_json.get("stream_url")
                        .and_then(Value::as_str)
                        .map(String::from),
                    hls_url: None, // Will be populated when needed
                    download_url: track_json.get("download_url")
                        .and_then(Value::as_str)
                        .map(String::from),
                    // Stats
                    playback_count: track_json.get("playback_count").and_then(Value::as_u64),
                    likes_count: track_json.get("likes_count").and_then(Value::as_u64),
                    reposts_count: track_json.get("reposts_count").and_then(Value::as_u64),
                    comment_count: track_json.get("comment_count").and_then(Value::as_u64),
                    // Additional metadata
                    genre: track_json.get("genre").and_then(Value::as_str).map(String::from),
                    tag_list: track_json.get("tag_list").and_then(Value::as_str).map(String::from),
                    downloadable: track_json.get("downloadable").and_then(Value::as_bool),
                    raw_data: Some(track_json.clone()),
                };
                tracks.push(track);
                batch_count += 1;
            } else {
                warn!("Track missing ID in API response - skipping");
            }
        }
        
        debug!("Added {} tracks from batch, total: {}", batch_count, tracks.len());
        
        offset += collection.len();
        
        // If we got fewer tracks than requested, there are no more
        if collection.len() < current_limit {
            debug!("Received fewer tracks than requested ({} < {}), no more available", 
                  collection.len(), current_limit);
            break;
        }
    }
    
    info!("Successfully fetched {} tracks for user {}", tracks.len(), user_id);
    Ok(tracks)
}

/// Get user details from SoundCloud
async fn get_user_details(user_id: &str) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    
    // Get the current client ID or refresh it
    let mut client_id = match get_client_id() {
        Some(id) => id,
        None => refresh_client_id().await?,
    };
    
    let max_retries = 3;
    let url = format!(
        "https://api-v2.soundcloud.com/users/{}?client_id={}",
        user_id, client_id
    );
    
    debug!("Fetching user details for user ID: {}", user_id);
    
    for retry in 0..max_retries {
        if retry > 0 {
            debug!("Retrying user details fetch (attempt {}/{}) for user {}", 
                  retry + 1, max_retries, user_id);
            sleep(Duration::from_secs(2 * retry as u64)).await;
        }
        
        let response = match client.get(&url).send().await {
            Ok(res) => {
                if !res.status().is_success() {
                    // Check for auth error and refresh client ID
                    if res.status().as_u16() == 401 || res.status().as_u16() == 403 {
                        warn!("Auth error ({}), refreshing client ID", res.status());
                        client_id = refresh_client_id().await?;
                        continue;
                    }
                    
                    warn!("API error: HTTP {} when fetching user details for {}", res.status(), user_id);
                    continue;
                }
                res
            }
            Err(e) => {
                warn!("Network error when fetching user details for {}: {}", user_id, e);
                continue;
            }
        };
        
        match response.json::<Value>().await {
            Ok(json) => {
                debug!("Successfully fetched user details for user {}", user_id);
                return Ok(json);
            }
            Err(e) => {
                warn!("JSON parse error for user details: {}", e);
                if retry == max_retries - 1 {
                    return Err(format!("Failed to parse JSON after {} retries", max_retries).into());
                }
            }
        }
    }
    
    Err(format!("Failed to fetch user details for {} after {} retries", user_id, max_retries).into())
}

// Parse user info from track JSON
fn parse_track_user(track_json: &Value) -> TrackUser {
    if let Some(user) = track_json.get("user") {
        TrackUser {
            id: user.get("id")
                .and_then(Value::as_u64)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            username: user.get("username")
                .and_then(Value::as_str)
                .unwrap_or("Unknown Artist")
                .to_string(),
            permalink_url: user.get("permalink_url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            avatar_url: user.get("avatar_url")
                .and_then(Value::as_str)
            .map(|url| get_original_artwork_url(url)),
        }
    } else {
        // Default user if not found
        TrackUser {
            id: String::new(),
            username: "Unknown Artist".to_string(),
            permalink_url: String::new(),
            avatar_url: None,
        }
    }
}

/// Get detailed information for a track including stream URLs
pub async fn get_track_details(
    track_id: &str
) -> Result<Track, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    
    // Get the current client ID or refresh it
    let mut client_id = match get_client_id() {
        Some(id) => id,
        None => refresh_client_id().await?,
    };
    
    let max_retries = 3;
    let mut json_response = None;
    
    for retry in 0..max_retries {
        if retry > 0 {
            debug!("Retrying track details fetch (attempt {}/{}) for track {}", 
                  retry + 1, max_retries, track_id);
            sleep(Duration::from_secs(2 * retry as u64)).await;
        }
        
        let url = format!(
            "https://api-v2.soundcloud.com/tracks/{}?client_id={}",
            track_id, client_id
        );
        
        let response = match client.get(&url).send().await {
            Ok(res) => {
                if !res.status().is_success() {
                    // Check for auth error and refresh client ID
                    if res.status().as_u16() == 401 || res.status().as_u16() == 403 {
                        warn!("Auth error ({}), refreshing client ID", res.status());
                        client_id = refresh_client_id().await?;
                        continue;
                    }
                    
                    warn!("API error: HTTP {} for track {}", res.status(), track_id);
                    continue;
                }
                res
            }
            Err(e) => {
                warn!("Request error for track {}: {}", track_id, e);
                continue;
            }
        };
        
        match response.json::<Value>().await {
            Ok(json) => {
                json_response = Some(json);
                break;
            }
            Err(e) => {
                warn!("JSON parse error for track {}: {}", track_id, e);
            }
        }
    }
    
    let json = match json_response {
        Some(j) => j,
        None => return Err(format!("Failed to fetch details for track {} after {} retries", 
                                  track_id, max_retries).into()),
    };
    
    // Basic track info
    let track = Track {
        id: track_id.to_string(),
        title: json.get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled")
            .to_string(),
        permalink_url: json.get("permalink_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        artwork_url: json.get("artwork_url")
            .and_then(Value::as_str)
            .map(|url| get_original_artwork_url(url)),
        description: json.get("description")
            .and_then(Value::as_str)
            .map(String::from),
        user: parse_track_user(&json),
        created_at: json.get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        duration: json.get("duration")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stream_url: json.get("stream_url")
            .and_then(Value::as_str)
            .map(String::from),
        download_url: json.get("download_url")
            .and_then(Value::as_str)
            .map(String::from),
        hls_url: None, // Populate this below if available
        // Stats
        playback_count: json.get("playback_count").and_then(Value::as_u64),
        likes_count: json.get("likes_count").and_then(Value::as_u64),
        reposts_count: json.get("reposts_count").and_then(Value::as_u64),
        comment_count: json.get("comment_count").and_then(Value::as_u64),
        // Additional metadata
        genre: json.get("genre").and_then(Value::as_str).map(String::from),
        tag_list: json.get("tag_list").and_then(Value::as_str).map(String::from),
        downloadable: json.get("downloadable").and_then(Value::as_bool),
        raw_data: Some(json.clone()),
    };
    
    info!("Fetched details for track {} - {}", track_id, track.title);
    
    // Try to extract HLS stream URL for the track
    if let Some(media) = json.get("media") {
        if let Some(transcodings) = media.get("transcodings").and_then(Value::as_array) {
            for transcoding in transcodings {
                let format = transcoding.get("format").and_then(|f| f.get("protocol")).and_then(Value::as_str);
                
                // Look for HLS streams specifically
                if let (Some("hls"), Some(url)) = (format, transcoding.get("url").and_then(Value::as_str)) {
                    debug!("Found HLS URL for track {}", track_id);
                    // TODO: Actually resolve the HLS URL by making another API call with client_id
                    // For now just return the original URL
                    let mut track = track.clone();
                    track.hls_url = Some(url.to_string());
                    return Ok(track);
                }
            }
        }
    }
    
    Ok(track)
}

/// Resolve the actual download/stream URL for a track
pub async fn get_stream_url(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    
    // Get the current client ID or refresh it
    let client_id = match get_client_id() {
        Some(id) => id,
        None => refresh_client_id().await?,
    };
    
    // Add client_id to URL
    let full_url = if url.contains('?') {
        format!("{}&client_id={}", url, client_id)
    } else {
        format!("{}?client_id={}", url, client_id)
    };
    
    let response = client.get(&full_url).send().await?;
    
    if !response.status().is_success() {
        return Err(format!("HTTP error {}", response.status()).into());
    }
    
    #[derive(Deserialize)]
    struct StreamResponse {
        url: String,
    }
    
    let stream_response: StreamResponse = response.json().await?;
    Ok(stream_response.url)
}

/// Resolve a SoundCloud URL to a track/user ID
pub async fn resolve_url(url: &str) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    
    // Get the current client ID or refresh it
    let mut client_id = match get_client_id() {
        Some(id) => id,
        None => refresh_client_id().await?,
    };
    
    let max_retries = 3;
    
    for retry in 0..max_retries {
        if retry > 0 {
            debug!("Retrying URL resolution (attempt {}/{}) for {}", 
                  retry + 1, max_retries, url);
            sleep(Duration::from_secs(2 * retry as u64)).await;
        }
        
        let resolve_url = format!(
            "https://api-v2.soundcloud.com/resolve?url={}&client_id={}",
            url, client_id
        );
        
        let response = match client.get(&resolve_url).send().await {
            Ok(res) => {
                if !res.status().is_success() {
                    // Check for auth error and refresh client ID
                    if res.status().as_u16() == 401 || res.status().as_u16() == 403 {
                        warn!("Auth error ({}), refreshing client ID", res.status());
                        client_id = refresh_client_id().await?;
                        continue;
                    }
                    
                    warn!("API error: HTTP {} for URL {}", res.status(), url);
                    continue;
                }
                res
            }
            Err(e) => {
                warn!("Request error for URL {}: {}", url, e);
                continue;
            }
        };
        
        match response.json::<Value>().await {
            Ok(json) => {
                info!("Successfully resolved URL: {}", url);
                return Ok(json);
            }
            Err(e) => {
                warn!("JSON parse error for URL {}: {}", url, e);
                if retry == max_retries - 1 {
                    return Err(format!("Failed to parse JSON after {} retries", max_retries).into());
                }
            }
        }
    }
    
    Err(format!("Failed to resolve URL {} after {} retries", url, max_retries).into())
}

/// Convert artwork URL to get the original high-resolution version
/// Example: https://i1.sndcdn.com/artworks-ABC123-y07N4g-large.jpg → https://i1.sndcdn.com/artworks-ABC123-y07N4g-original.jpg
pub fn get_original_artwork_url(artwork_url: &str) -> String {
    // If the URL contains '-large.jpg', replace it with '-original.jpg'
    if artwork_url.contains("-large.jpg") {
        return artwork_url.replace("-large.jpg", "-original.jpg");
    }
    
    // If the URL contains '-t500x500.jpg', replace it with '-original.jpg' (older format)
    if artwork_url.contains("-t500x500.jpg") {
        return artwork_url.replace("-t500x500.jpg", "-original.jpg");
    }
    
    // Return the original URL if it doesn't match the expected pattern
    artwork_url.to_string()
}

/// Get a list of users that a SoundCloud user is following
pub async fn get_user_followings(
    user_id: &str, 
    limit: Option<usize>
) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    let mut followings = Vec::new();
    let mut offset = 0;
    // API has a max limit of 200 per request
    let chunk_size = 200;
    let max_limit = limit.unwrap_or(usize::MAX);
    
    info!("Fetching followings for user {}", user_id);
    
    // Get the current client ID or refresh it
    let mut client_id = match get_client_id() {
        Some(id) => {
            debug!("Using cached client ID: {}", id);
            id
        },
        None => {
            debug!("No cached client ID, generating new one");
            refresh_client_id().await?
        },
    };
    
    loop {
        // Break if we've reached the requested limit
        if followings.len() >= max_limit {
            break;
        }
        
        let current_limit = std::cmp::min(chunk_size, max_limit - followings.len());
        let url = format!(
            "https://api-v2.soundcloud.com/users/{}/followings?client_id={}&limit={}&offset={}&linked_partitioning=1",
            user_id, client_id, current_limit, offset
        );
        
        debug!("Fetching followings batch: offset={}, limit={}", offset, current_limit);
        
        // Make the request with retry logic
        let mut response_json = None;
        let max_retries = 3;
        
        for retry in 0..max_retries {
            if retry > 0 {
                debug!("Retrying followings fetch (attempt {}/{}) for user {}", 
                      retry + 1, max_retries, user_id);
                sleep(Duration::from_secs(2 * retry as u64)).await;
            }
            
            let response = match client.get(&url).send().await {
                Ok(res) => {
                    if !res.status().is_success() {
                        // Check for auth error and refresh client ID
                        if res.status().as_u16() == 401 || res.status().as_u16() == 403 {
                            warn!("Auth error ({}), refreshing client ID", res.status());
                            client_id = refresh_client_id().await?;
                            continue;
                        }
                        
                        warn!("API error: HTTP {} when fetching followings for user {}", res.status(), user_id);
                        continue;
                    }
                    res
                }
                Err(e) => {
                    warn!("Network error when fetching followings for user {}: {}", user_id, e);
                    continue;
                }
            };
            
            match response.json::<Value>().await {
                Ok(json) => {
                    response_json = Some(json);
                    break;
                }
                Err(e) => {
                    warn!("JSON parse error for followings response: {}", e);
                    if retry == max_retries - 1 {
                        return Err(format!("Failed to parse JSON after {} retries", max_retries).into());
                    }
                }
            }
        }
        
        let json = match response_json {
            Some(j) => j,
            None => {
                error!("Failed to fetch followings for user {} after {} retries", user_id, max_retries);
                return Err(format!("Failed to fetch followings for user {} after {} retries", 
                                  user_id, max_retries).into());
            }
        };
        
        // Extract the collection of followings
        let collection = match json.get("collection") {
            Some(Value::Array(arr)) => arr,
            _ => {
                error!("Unexpected API response format for user {}: missing 'collection' array", user_id);
                return Err(format!("Unexpected API response format for user {}", user_id).into());
            }
        };
        
        if collection.is_empty() {
            debug!("No more followings found for user {} at offset {}", user_id, offset);
            break; // No more followings
        }
        
        debug!("Processing {} followings from response", collection.len());
        
        // Add followings to our collection
        for following in collection {
            followings.push(following.clone());
        }
        
        debug!("Added {} followings from batch, total: {}", collection.len(), followings.len());
        
        // Check if there are more pages
        if let Some(next_href) = json.get("next_href").and_then(Value::as_str) {
            // Extract offset from next_href
            if let Some(new_offset) = extract_offset_from_url(next_href) {
                offset = new_offset;
                debug!("Next page available, offset: {}", offset);
            } else {
                // Can't extract offset, so just increment by collection size
                offset += collection.len();
                debug!("Couldn't extract offset from next_href, incrementing by collection size");
            }
        } else {
            debug!("No next_href found, this is the last page");
            break;
        }
    }
    
    info!("Successfully fetched {} followings for user {}", followings.len(), user_id);
    Ok(followings)
}

/// Extract offset parameter from a SoundCloud API URL
fn extract_offset_from_url(url: &str) -> Option<usize> {
    if let Some(query) = url.split('?').nth(1) {
        for param in query.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                if key == "offset" {
                    return value.parse::<usize>().ok();
                }
            }
        }
    }
    None
} 