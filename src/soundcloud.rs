use std::sync::Mutex;
use std::time::Duration;
use log::{info, warn, error, debug};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;
use std::sync::Arc;

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

/// Like structure returned from the SoundCloud API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Like {
    pub created_at: String,
    pub kind: String,
    pub track: Track,
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
    _pagination_size: usize, // Keep parameter for backward compatibility
) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    let mut tracks = Vec::new();
    let mut seen_track_ids = std::collections::HashSet::new();
    
    // First, get the user's details to check for total track count
    let user_data = match get_user_details(user_id).await {
        Ok(data) => data,
        Err(e) => {
            warn!("Failed to get user details for {}: {}. Using configured limit.", user_id, e);
            return Ok(Vec::new());
        }
    };
    
    let total_tracks = match user_data.get("track_count").and_then(|v| v.as_u64()) {
        Some(count) => count as usize,
        None => {
            warn!("Could not determine track count for user {}, using configured limit", user_id);
            return Ok(Vec::new());
        }
    };
    
    info!("User {} has {} tracks according to their profile", user_id, total_tracks);
    
    // Use the smaller of the configured limit or actual track count
    let effective_limit = limit;
    info!("Will fetch up to {} tracks", effective_limit);
    
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
    
    // Try to fetch all tracks in one go with a large limit
    let url = format!(
        "https://api-v2.soundcloud.com/users/{}/tracks?client_id={}&limit={}&linked_partitioning=1",
        user_id, client_id, effective_limit
    );
    
    debug!("Attempting to fetch all {} tracks in one request", effective_limit);
    
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
        debug!("No tracks found for user {}", user_id);
        return Ok(Vec::new());
    }
    
    debug!("Processing {} tracks from response", collection.len());
    
    // Parse the tracks
    let mut batch_count = 0;
    for track_json in collection {
        // Extract basic fields
        if let Some(id) = track_json.get("id").and_then(Value::as_u64) {
            let track_id = id.to_string();
            
            // Skip if we've already seen this track
            if !seen_track_ids.insert(track_id.clone()) {
                debug!("Skipping duplicate track ID: {}", track_id);
                continue;
            }
            
            let title = track_json.get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled")
                .to_string();
            
            debug!("Processing track: {} (ID: {})", title, id);
            
            let track = Track {
                id: track_id,
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

/// Get likes for a SoundCloud user
pub async fn get_user_likes(
    user_id: &str, 
    limit: usize,
    _pagination_size: usize, // Keep parameter for backward compatibility
) -> Result<Vec<Like>, Box<dyn std::error::Error + Send + Sync>> {
    let client = &HTTP_CLIENT;
    let mut likes = Vec::new();
    let mut seen_like_ids = std::collections::HashSet::new();
    
    info!("Fetching up to {} likes for user {}", limit, user_id);
    
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
    
    // Try to fetch all likes in one go with a large limit
    let url = format!(
        "https://api-v2.soundcloud.com/users/{}/likes?client_id={}&limit={}&linked_partitioning=1",
        user_id, client_id, limit
    );
    
    debug!("Attempting to fetch all {} likes in one request", limit);
    
    // Make the request with retry logic
    let mut response_json = None;
    let max_retries = 3;
    
    for retry in 0..max_retries {
        if retry > 0 {
            debug!("Retrying likes fetch (attempt {}/{}) for user {}", 
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
                    
                    warn!("API error: HTTP {} when fetching likes for user {}", res.status(), user_id);
                    continue;
                }
                res
            }
            Err(e) => {
                warn!("Network error when fetching likes for user {}: {}", user_id, e);
                continue;
            }
        };
        
        match response.json::<Value>().await {
            Ok(json) => {
                response_json = Some(json);
                break;
            }
            Err(e) => {
                warn!("JSON parse error for likes response: {}", e);
                if retry == max_retries - 1 {
                    return Err(format!("Failed to parse JSON after {} retries", max_retries).into());
                }
            }
        }
    }
    
    let json = match response_json {
        Some(j) => j,
        None => {
            error!("Failed to fetch likes for user {} after {} retries", user_id, max_retries);
            return Err(format!("Failed to fetch likes for user {} after {} retries", 
                              user_id, max_retries).into());
        }
    };
    
    // Extract the collection of likes
    let collection = match json.get("collection") {
        Some(Value::Array(arr)) => arr,
        _ => {
            error!("Unexpected API response format for user {}: missing 'collection' array", user_id);
            return Err(format!("Unexpected API response format for user {}", user_id).into());
        }
    };
    
    if collection.is_empty() {
        debug!("No likes found for user {}", user_id);
        return Ok(Vec::new());
    }
    
    debug!("Processing {} likes from response", collection.len());
    
    // Parse the likes
    let mut batch_count = 0;
    for like_json in collection {
        // Each like contains a track
        if let Some(track_json) = like_json.get("track") {
            if let Some(kind) = like_json.get("kind").and_then(Value::as_str) {
                if kind == "like" {
                    // Parse the created_at date
                    let created_at = like_json.get("created_at")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    
                    // Extract track
                    if let Some(id) = track_json.get("id").and_then(Value::as_u64) {
                        let track_id = id.to_string();
                        
                        // Skip if we've already seen this like
                        if !seen_like_ids.insert(track_id.clone()) {
                            debug!("Skipping duplicate like for track ID: {}", track_id);
                            continue;
                        }
                        
                        let title = track_json.get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("Untitled")
                            .to_string();
                        
                        debug!("Processing liked track: {} (ID: {})", title, id);
                        
                        let track = Track {
                            id: track_id,
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
                        
                        // Create the like structure
                        let like = Like {
                            created_at,
                            kind: kind.to_string(),
                            track,
                        };
                        
                        likes.push(like);
                        batch_count += 1;
                    }
                }
            }
        }
    }
    
    debug!("Added {} likes from batch, total: {}", batch_count, likes.len());
    
    info!("Successfully fetched {} likes for user {}", likes.len(), user_id);
    Ok(likes)
}

/// Extract tracks from user likes
pub fn extract_tracks_from_likes(likes: &[Like]) -> Vec<Track> {
    let tracks: Vec<Track> = likes
        .iter()
        .map(|like| like.track.clone())
        .collect();
    
    debug!("Extracted {} tracks from {} likes", tracks.len(), likes.len());
    tracks
}

/// Display information about a SoundCloud URL
/// 
/// Resolves a SoundCloud URL and displays formatted information about it.
/// This function encapsulates the SoundCloud URL resolution logic that was previously in main.rs.
pub async fn display_soundcloud_info(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Resolving SoundCloud URL: {}", url);
    
    // Resolve the URL
    info!("Fetching metadata from SoundCloud API");
    let resolved = match resolve_url(url).await {
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
                let track = get_track_details(&track_id).await?;
                
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

/// Process and post a single track to Discord
/// 
/// Takes either a track ID or URL, resolves it, processes the audio, and posts to Discord.
/// Returns the Discord message ID and track ID for further processing.
pub async fn process_and_post_track(
    id_or_url: &str,
    discord_webhook_url: &str,
    temp_dir: Option<&str>,
    discord_semaphore: Option<&Arc<tokio::sync::Semaphore>>
) -> Result<(String, String, crate::discord::WebhookResponse), Box<dyn std::error::Error + Send + Sync>> {
    // Check if this is a URL or an ID
    let track_id = if id_or_url.starts_with("http") {
        // This is a URL, resolve it
        info!("Resolving SoundCloud URL: {}", id_or_url);
        let resolved = match resolve_url(id_or_url).await {
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
    let track_details = match get_track_details(&track_id).await {
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
    let processing_result = match crate::audio::process_track_audio(&track_details, temp_dir).await {
        Ok((audio_files, artwork, json)) => {
            let mut files = Vec::new();
            
            // Process all audio files
            for (format_info, path) in &audio_files {
                let file_path = path.clone();
                let filename = std::path::Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("track.audio"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Audio file ({}): {}", format_info, filename);
                files.push((file_path, filename));
            }
            
            if let Some(path) = artwork {
                let file_path = path.clone();
                let filename = std::path::Path::new(&file_path)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("cover.jpg"))
                    .to_string_lossy()
                    .to_string();
                
                info!("Downloaded artwork: {}", filename);
                files.push((file_path, filename));
            }
            
            if let Some(path) = json {
                let file_path = path.clone();
                let filename = std::path::Path::new(&file_path)
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
    
    // Acquire Discord semaphore if provided
    let _discord_permit = if let Some(semaphore) = discord_semaphore {
        match semaphore.acquire().await {
            Ok(permit) => Some(permit),
            Err(e) => {
                error!("Failed to acquire Discord semaphore for track {}: {}", track_id, e);
                return Err(format!("Failed to acquire Discord semaphore: {}", e).into());
            }
        }
    } else {
        None
    };
    
    let webhook_response = match crate::discord::send_track_webhook(discord_webhook_url, &track_details, Some(processing_result.clone())).await {
        Ok(response) => {
            info!("Successfully sent webhook for track with message ID: {}", response.message_id);
            println!("Track successfully posted to Discord: {} by {}", 
                   track_details.title, track_details.user.username);
            println!("Discord message ID: {}", response.message_id);
            response
        },
        Err(e) => {
            error!("Failed to send webhook: {}", e);
            return Err(e);
        }
    };
    
    // Clean up temp files
    for (path, _) in processing_result.clone() {
        if let Err(e) = crate::audio::delete_temp_file(&path).await {
            warn!("Failed to clean up temp file {}: {}", path, e);
        }
    }
    
    Ok((track_id, track_details.user.id.clone(), webhook_response))
} 