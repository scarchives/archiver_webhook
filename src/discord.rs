use reqwest::{Client, multipart};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use log::{info, warn, error, debug};
use crate::soundcloud::Track;

/// Send a track to Discord via webhook
pub async fn send_track_webhook(
    webhook_url: &str, 
    track: &Track,
    audio_files: Option<Vec<(String, String)>> // Vec of (file_path, file_name)
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create the webhook client
    let client = Client::new();
    
    // Build the embed object
    info!("Preparing Discord webhook for track '{}' (ID: {})", track.title, track.id);
    let embed = build_track_embed(track);
    
    // Check audio files
    let files_count = match &audio_files {
        Some(files) => files.len(),
        None => 0,
    };
    
    // If we have audio files, we need to use multipart/form-data
    // Otherwise, we can just use a simple JSON post
    let result = if let Some(files) = audio_files {
        if files.is_empty() {
            debug!("No audio files attached, sending embed only");
            send_embed_only(client, webhook_url, embed).await
        } else {
            debug!("Attaching {} audio files to webhook", files.len());
            send_with_audio_files(client, webhook_url, embed, files).await
        }
    } else {
        debug!("No audio files provided, sending embed only");
        send_embed_only(client, webhook_url, embed).await
    };
    
    // Log result
    match &result {
        Ok(_) => info!("Successfully sent Discord webhook for track '{}' with {} audio files", 
                      track.title, files_count),
        Err(e) => error!("Failed to send Discord webhook for track '{}': {}", track.title, e),
    }
    
    result
}

/// Build a Discord embed for the track
fn build_track_embed(track: &Track) -> Value {
    debug!("Building Discord embed for track '{}' (ID: {})", track.title, track.id);
    
    // Extract additional metadata from raw_data if available
    let description = track.description.clone().unwrap_or_default();
    
    // These values will be populated from either raw_data or track struct directly
    let play_count: Option<u64>;
    let likes_count: Option<u64>;
    let reposts_count: Option<u64>;
    let comment_count: Option<u64>;
    let genre: Option<String>;
    let tags: Option<String>;
    let downloadable: bool;
    
    if let Some(raw_data) = &track.raw_data {
        // Get play count
        play_count = raw_data.get("playback_count").and_then(|v| v.as_u64());
        
        // Get likes count
        likes_count = raw_data.get("likes_count").and_then(|v| v.as_u64());
        
        // Get reposts count
        reposts_count = raw_data.get("reposts_count").and_then(|v| v.as_u64());
        
        // Get comment count
        comment_count = raw_data.get("comment_count").and_then(|v| v.as_u64());
        
        // Get genre
        genre = raw_data.get("genre").and_then(|v| v.as_str()).map(String::from);
        
        // Get tags
        tags = raw_data.get("tag_list").and_then(|v| v.as_str()).map(String::from);
        
        // Check if downloadable
        downloadable = raw_data.get("downloadable").and_then(|v| v.as_bool()).unwrap_or(false);
    } else {
        // Use values from the track struct directly if available
        play_count = track.playback_count;
        likes_count = track.likes_count;
        reposts_count = track.reposts_count;
        comment_count = track.comment_count;
        genre = track.genre.clone();
        tags = track.tag_list.clone();
        downloadable = track.downloadable.unwrap_or(false);
    }
    
    debug!("Track metadata - plays: {:?}, likes: {:?}, reposts: {:?}, comments: {:?}", 
           play_count, likes_count, reposts_count, comment_count);
    
    // Build fields for the embed
    let mut fields = vec![];
    
    // Add genre if available
    if let Some(g) = genre {
        if !g.is_empty() {
            fields.push(json!({
                "name": "Genre",
                "value": g,
                "inline": true
            }));
        }
    }
    
    // Add tags as a separate field if available
    if let Some(tag_list) = tags {
        if !tag_list.is_empty() {
            let parsed_tags = parse_tags(&tag_list);
            if !parsed_tags.is_empty() {
                fields.push(json!({
                    "name": "Tags",
                    "value": parsed_tags.join(", "),
                    "inline": false
                }));
            }
        }
    }
    
    debug!("Created {} embed fields for Discord message", fields.len());
    
    // Get original high-resolution artwork URL if available
    let artwork_url = track.artwork_url.clone()
        .map(|url| crate::soundcloud::get_original_artwork_url(&url))
        .unwrap_or_default();
    
    // Create the embed object
    json!({
        "title": track.title,
        "type": "rich",
        "description": description,
        "url": track.permalink_url,
        "timestamp": track.created_at,
        "color": 0xFF7700, // SoundCloud orange
        "author": {
            "name": track.user.username.clone(),
            "url": track.user.permalink_url.clone(),
            "icon_url": track.user.avatar_url.clone().unwrap_or_default()
        },
        "thumbnail": {
            "url": artwork_url
        },
        "fields": fields
    })
}

/// Parse a tag list string, respecting quoted tags
/// 
/// Handles:
/// - Space-separated individual tags
/// - Tags enclosed in double quotes (treated as a single tag)
/// - Supports nested quotes
fn parse_tags(tag_list: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut current_tag = String::new();
    let mut in_quotes = false;
    let mut escape_next = false;
    
    for c in tag_list.chars() {
        match (c, in_quotes, escape_next) {
            // Handle escape character
            ('\\', _, false) => {
                escape_next = true;
            },
            // Start or end quote
            ('"', _, true) => {
                current_tag.push('"');
                escape_next = false;
            },
            ('"', false, false) => {
                in_quotes = true;
            },
            ('"', true, false) => {
                in_quotes = false;
            },
            // Space handling
            (' ', false, false) => {
                if !current_tag.is_empty() {
                    tags.push(current_tag);
                    current_tag = String::new();
                }
            },
            // Regular character
            (_, _, true) => {
                current_tag.push('\\');
                current_tag.push(c);
                escape_next = false;
            },
            (_, _, false) => {
                current_tag.push(c);
            }
        }
    }
    
    // Don't forget the last tag if there is one
    if !current_tag.is_empty() {
        tags.push(current_tag);
    }
    
    tags
}

/// Send just the embed without any files
async fn send_embed_only(
    client: Client, 
    webhook_url: &str, 
    embed: Value
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Preparing embed-only Discord webhook request");
    
    let payload = json!({
        "embeds": [embed],
        "username": "SoundCloud Archiver",
    });
    
    debug!("Sending webhook POST request to Discord");
    let response = client
        .post(webhook_url)
        .json(&payload)
        .send()
        .await?;
    
    let status = response.status();
    debug!("Discord API response status: {}", status);
    
    if !status.is_success() {
        let error_text = response.text().await?;
        error!("Discord webhook error: {} - {}", status, error_text);
        return Err(format!("Discord webhook error: {} - {}", status, error_text).into());
    }
    
    debug!("Discord webhook sent successfully");
    Ok(())
}

/// Send the embed with audio file attachments
async fn send_with_audio_files(
    client: Client,
    webhook_url: &str,
    embed: Value,
    files: Vec<(String, String)> // Vec of (file_path, file_name)
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Preparing multipart request with {} audio files", files.len());
    
    // Create a multipart form
    let mut form = multipart::Form::new()
        .text("payload_json", json!({
            "embeds": [embed],
            "username": "SoundCloud Archiver",
        }).to_string());
    
    // Add each audio file
    for (i, (file_path, file_name)) in files.iter().enumerate() {
        // Read the file
        debug!("Adding file {}/{} to multipart form: {}", i+1, files.len(), file_name);
        
        let path = Path::new(file_path);
        let file_size = match fs::metadata(path) {
            Ok(metadata) => metadata.len(),
            Err(e) => {
                warn!("Failed to get file size for {}: {}", file_path, e);
                0
            }
        };
        
        let mut file = match File::open(path).await {
            Ok(f) => {
                debug!("Opened file: {} ({} bytes)", file_path, file_size);
                f
            },
            Err(e) => {
                error!("Failed to open file {}: {}", file_path, e);
                return Err(format!("Failed to open file {}: {}", file_path, e).into());
            }
        };
        
        let mut buffer = Vec::new();
        match file.read_to_end(&mut buffer).await {
            Ok(size) => debug!("Read {} bytes from file {}", size, file_path),
            Err(e) => {
                error!("Failed to read file {}: {}", file_path, e);
                return Err(format!("Failed to read file {}: {}", file_path, e).into());
            }
        }
        
        // Determine MIME type
        let mime_type = match path.extension() {
            Some(ext) if ext == "mp3" => "audio/mpeg",
            Some(ext) if ext == "ogg" => "audio/ogg",
            Some(ext) if ext == "opus" => "audio/opus",
            Some(ext) if ext == "m4a" => "audio/mp4",
            Some(ext) => {
                let ext_str = ext.to_string_lossy();
                debug!("Unknown extension '{}', using default MIME type", ext_str);
                "application/octet-stream"
            }
            None => {
                debug!("No file extension, using default MIME type");
                "application/octet-stream"
            }
        };
        
        // Add to form
        debug!("Adding part to form: file{} as {} (MIME: {})", i, file_name, mime_type);
        let part = multipart::Part::bytes(buffer)
            .file_name(file_name.clone())
            .mime_str(mime_type)?;
        form = form.part(format!("file{}", i), part);
    }
    
    // Send the form
    debug!("Sending multipart POST request to Discord webhook");
    let response = client
        .post(webhook_url)
        .multipart(form)
        .send()
        .await?;
    
    let status = response.status();
    debug!("Discord API response status: {}", status);
    
    if !status.is_success() {
        let error_text = response.text().await?;
        error!("Discord webhook error: {} - {}", status, error_text);
        return Err(format!("Discord webhook error: {} - {}", status, error_text).into());
    }
    
    debug!("Discord webhook with files sent successfully");
    Ok(())
} 