use reqwest::{Client, multipart};
use serde_json::{json, Value};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use log::{info, warn, error};
use crate::soundcloud::Track;

/// Send a track to Discord via webhook
pub async fn send_track_webhook(
    webhook_url: &str, 
    track: &Track,
    audio_files: Option<Vec<(String, String)>> // Vec of (file_path, file_name)
) -> Result<(), Box<dyn std::error::Error>> {
    // Create the webhook client
    let client = Client::new();
    
    // Build the embed object
    let embed = build_track_embed(track);
    
    // If we have audio files, we need to use multipart/form-data
    // Otherwise, we can just use a simple JSON post
    if let Some(files) = audio_files {
        if files.is_empty() {
            send_embed_only(client, webhook_url, embed).await
        } else {
            send_with_audio_files(client, webhook_url, embed, files).await
        }
    } else {
        send_embed_only(client, webhook_url, embed).await
    }
}

/// Build a Discord embed for the track
fn build_track_embed(track: &Track) -> Value {
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
    }
    
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
            "icon_url": "https://developers.soundcloud.com/assets/logo_black.png"
        },
        "author": {
            "name": track.user.username.clone(),
            "url": track.user.permalink_url.clone(),
            "icon_url": track.user.avatar_url.clone().unwrap_or_default()
        },
        "thumbnail": {
            "url": track.artwork_url.clone().unwrap_or_default()
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
    let payload = json!({
        "embeds": [embed],
        "username": "SoundCloud Archiver",
        "avatar_url": "https://developers.soundcloud.com/assets/logo_big_orange.png"
    });
    
    let response = client
        .post(webhook_url)
        .json(&payload)
        .send()
        .await?;
    
    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await?;
        return Err(format!("Discord webhook error: {} - {}", status, error_text).into());
    }
    
    info!("Successfully sent Discord webhook for track");
    Ok(())
}

/// Send the embed with audio file attachments
async fn send_with_audio_files(
    client: Client,
    webhook_url: &str,
    embed: Value,
    files: Vec<(String, String)> // Vec of (file_path, file_name)
) -> Result<(), Box<dyn std::error::Error>> {
    // Create a multipart form
    let mut form = multipart::Form::new()
        .text("payload_json", json!({
            "embeds": [embed],
            "username": "SoundCloud Archiver",
            "avatar_url": "https://developers.soundcloud.com/assets/logo_big_orange.png"
        }).to_string());
    
    // Add each audio file
    for (i, (file_path, file_name)) in files.iter().enumerate() {
        // Read the file
        let path = Path::new(file_path);
        let mut file = File::open(path).await?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await?;
        
        // Add to form
        form = form.part(format!("file{}", i), 
            multipart::Part::bytes(buffer)
                .file_name(file_name.clone())
                .mime_str(match path.extension() {
                    Some(ext) if ext == "mp3" => "audio/mpeg",
                    Some(ext) if ext == "ogg" => "audio/ogg",
                    Some(ext) if ext == "opus" => "audio/opus",
                    Some(ext) if ext == "m4a" => "audio/mp4",
                    _ => "application/octet-stream"
                })?
        );
    }
    
    // Send the form
    let response = client
        .post(webhook_url)
        .multipart(form)
        .send()
        .await?;
    
    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await?;
        return Err(format!("Discord webhook error: {} - {}", status, error_text).into());
    }
    
    info!("Successfully sent Discord webhook with {} audio files", files.len());
    Ok(())
} 