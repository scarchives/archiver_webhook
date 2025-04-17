use reqwest::{Client, multipart};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use log::{info, warn, error, debug};
use crate::soundcloud::{Track, get_original_artwork_url};

/// Send a track to Discord via webhook
pub async fn send_track_webhook(
    webhook_url: &str, 
    track: &Track,
    audio_files: Option<Vec<(String, String)>> // Vec of (file_path, file_name)
) -> Result<(), Box<dyn std::error::Error>> {
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
    
    // Format the track duration
    let duration_secs = track.duration / 1000; // Convert from milliseconds
    let duration_str = format!(
        "{}:{:02}", 
        duration_secs / 60, 
        duration_secs % 60
    );
    
    // Extract additional metadata from raw_data if available
    let mut description = track.description.clone().unwrap_or_default();
    let mut play_count = None;
    let mut likes_count = None;
    let mut reposts_count = None;
    let mut comment_count = None;
    let mut genre = None;
    let mut tags = None;
    let mut downloadable = false;
    
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
    let mut fields = vec![
        json!({
            "name": "Duration",
            "value": duration_str,
            "inline": true
        })
    ];
    
    // Add play count if available
    if let Some(count) = play_count {
        fields.push(json!({
            "name": "Plays",
            "value": format!("{}", count),
            "inline": true
        }));
    }
    
    // Add likes count if available
    if let Some(count) = likes_count {
        fields.push(json!({
            "name": "Likes",
            "value": format!("{}", count),
            "inline": true
        }));
    }
    
    // Add reposts count if available
    if let Some(count) = reposts_count {
        fields.push(json!({
            "name": "Reposts",
            "value": format!("{}", count),
            "inline": true
        }));
    }
    
    // Add comments count if available
    if let Some(count) = comment_count {
        fields.push(json!({
            "name": "Comments",
            "value": format!("{}", count),
            "inline": true
        }));
    }
    
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
            fields.push(json!({
                "name": "Tags",
                "value": tag_list.replace(" ", ", "),
                "inline": false
            }));
        }
    }
    
    // Add download status
    fields.push(json!({
        "name": "Downloadable",
        "value": if downloadable { "Yes" } else { "No" },
        "inline": true
    }));
    
    debug!("Created {} embed fields for Discord message", fields.len());
    
    // Get original high-resolution artwork URL if available
    let artwork_url = track.artwork_url.clone().unwrap_or_default();
    let original_artwork_url = if !artwork_url.is_empty() {
        get_original_artwork_url(&artwork_url)
    } else {
        artwork_url
    };
    
    if !original_artwork_url.is_empty() {
        debug!("Using original artwork URL: {}", original_artwork_url);
    }
    
    // Create the embed object
    json!({
        "title": track.title,
        "type": "rich",
        "description": description,
        "url": track.permalink_url,
        "timestamp": track.created_at,
        "color": 0xFF7700, // SoundCloud orange
        "footer": {
            "text": "Archived via SoundCloud Archiver",
            "icon_url": "https://developers.soundcloud.com/assets/logo_big_white-65c2b096da68dd533db18b9f07d14054.png"
        },
        "author": {
            "name": track.user.username.clone(),
            "url": track.user.permalink_url.clone(),
            "icon_url": track.user.avatar_url.clone().unwrap_or_default()
        },
        "thumbnail": {
            "url": original_artwork_url
        },
        "fields": fields
    })
}

/// Send just the embed without any files
async fn send_embed_only(
    client: Client, 
    webhook_url: &str, 
    embed: Value
) -> Result<(), Box<dyn std::error::Error>> {
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
) -> Result<(), Box<dyn std::error::Error>> {
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
        match multipart::Part::bytes(buffer)
            .file_name(file_name.clone())
            .mime_str(mime_type)
        {
            Ok(part) => {
                form = form.part(format!("file{}", i), part);
            },
            Err(e) => {
                error!("Failed to create multipart part: {}", e);
                return Err(e.into());
            }
        }
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