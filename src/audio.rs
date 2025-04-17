use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::env;
use log::{info, warn, error, debug};
use tokio::process::Command as TokioCommand;
use tokio::fs::File as TokioFile;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use crate::soundcloud::{Track, get_stream_url};
use lazy_static;

/// Download and transcode audio from a SoundCloud track
/// Returns paths to the MP3, OGG, Artwork, and JSON metadata files
pub async fn process_track_audio(
    track: &Track,
    temp_dir: Option<&str>
) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    // Get the base temp directory
    let base_dir = match temp_dir {
        Some(dir) => {
            debug!("Using specified temp directory: {}", dir);
            PathBuf::from(dir)
        },
        None => {
            let sys_temp = env::temp_dir();
            debug!("Using system temp directory: {}", sys_temp.display());
            sys_temp
        },
    };
    
    // Create a unique subfolder for this download
    let unique_id = Uuid::new_v4().to_string();
    let work_dir = base_dir.join(format!("scarchive_{}", unique_id));
    debug!("Creating work directory: {}", work_dir.display());
    fs::create_dir_all(&work_dir)?;
    
    info!("Processing audio for track '{}' (ID: {}) in {}", track.title, track.id, work_dir.display());
    
    // Save raw track data as JSON
    let mut json_result = None;
    let sanitized_title = sanitize_filename(&track.title);
    let json_path = work_dir.join(format!("{}_data.json", sanitized_title));
    
    match save_track_json(track, &json_path).await {
        Ok(()) => {
            let file_size = match fs::metadata(&json_path) {
                Ok(metadata) => metadata.len(),
                Err(_) => 0,
            };
            
            json_result = Some(json_path.to_string_lossy().to_string());
            info!("Saved track data as JSON: {} ({} bytes)", json_path.display(), file_size);
        },
        Err(e) => {
            warn!("Failed to save track data as JSON: {}", e);
        }
    }
    
    // Resolve the HLS URL if we have one
    let hls_url = match &track.hls_url {
        Some(url) => {
            debug!("Resolving HLS URL for track {}", track.id);
            match get_stream_url(url).await {
                Ok(resolved) => {
                    info!("Successfully resolved HLS URL for track {}", track.id);
                    Some(resolved)
                },
                Err(e) => {
                    warn!("Failed to resolve HLS URL for track {}: {}", track.id, e);
                    None
                }
            }
        },
        None => {
            warn!("No HLS URL available for track {}, checking other streams", track.id);
            None
        }
    };
    
    // Resolve the stream URL if we have one (and no HLS)
    let stream_url = if hls_url.is_none() {
        match &track.stream_url {
            Some(url) => {
                debug!("Resolving stream URL for track {}", track.id);
                match get_stream_url(url).await {
                    Ok(resolved) => {
                        info!("Successfully resolved stream URL for track {}", track.id);
                        Some(resolved)
                    },
                    Err(e) => {
                        warn!("Failed to resolve stream URL for track {}: {}", track.id, e);
                        None
                    }
                }
            },
            None => {
                warn!("No stream URL available for track {}", track.id);
                None
            }
        }
    } else {
        None
    };
    
    // Download artwork if available
    let mut artwork_result = None;
    if let Some(artwork_url) = &track.artwork_url {
        if !artwork_url.is_empty() {
            // Get the original high-res image URL
            info!("Downloading original artwork from: {}", artwork_url);
            
            // Create file path for artwork
            let artwork_path = work_dir.join(format!("{}_cover.jpg", sanitized_title));
            
            // Download the artwork
            match download_artwork(&artwork_url, &artwork_path).await {
                Ok(()) => {
                    let file_size = match fs::metadata(&artwork_path) {
                        Ok(metadata) => metadata.len(),
                        Err(_) => 0,
                    };
                    
                    artwork_result = Some(artwork_path.to_string_lossy().to_string());
                    info!("Successfully downloaded artwork: {} ({} bytes)", artwork_path.display(), file_size);
                },
                Err(e) => {
                    warn!("Failed to download artwork: {}", e);
                }
            }
        }
    }
    
    // If we have neither audio stream, return error
    if hls_url.is_none() && stream_url.is_none() && json_result.is_none() && artwork_result.is_none() {
        error!("No valid audio URLs or data found for track {}", track.id);
        cleanup_temp_dir(&work_dir).await?;
        return Err("No valid audio URLs or data found for track".into());
    }
    
    // File paths for transcoded files
    let mp3_path = work_dir.join(format!("{}.mp3", sanitized_title));
    let ogg_path = work_dir.join(format!("{}.ogg", sanitized_title));
    
    debug!("Output file paths: MP3={}, OGG={}", mp3_path.display(), ogg_path.display());
    
    // Try to transcode to both formats
    let mut mp3_result = None;
    let mut ogg_result = None;
    
    // Process MP3 (preferring HLS if available)
    if let Some(url) = hls_url.as_ref().or(stream_url.as_ref()) {
        info!("Starting MP3 transcoding for track {}", track.id);
        debug!("Input URL: {}", url);
        debug!("Output path: {}", mp3_path.display());
        
        match transcode_to_mp3(url, &mp3_path).await {
            Ok(_) => {
                let file_size = match fs::metadata(&mp3_path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => 0,
                };
                
                mp3_result = Some(mp3_path.to_string_lossy().to_string());
                info!("Successfully transcoded to MP3: {} ({} bytes)", mp3_path.display(), file_size);
            },
            Err(e) => {
                error!("Failed to transcode to MP3: {}", e);
            }
        }
    } else {
        warn!("No URL available for MP3 transcoding");
    }
    
    // Process OGG (only from HLS for better quality)
    if let Some(url) = &hls_url {
        info!("Starting OGG transcoding for track {}", track.id);
        debug!("Input URL: {}", url);
        debug!("Output path: {}", ogg_path.display());
        
        match transcode_to_ogg(url, &ogg_path).await {
            Ok(_) => {
                let file_size = match fs::metadata(&ogg_path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => 0,
                };
                
                ogg_result = Some(ogg_path.to_string_lossy().to_string());
                info!("Successfully transcoded to OGG: {} ({} bytes)", ogg_path.display(), file_size);
            },
            Err(e) => {
                error!("Failed to transcode to OGG: {}", e);
            }
        }
    } else {
        warn!("No HLS URL available for OGG transcoding");
    }
    
    // If everything failed, return error
    if mp3_result.is_none() && ogg_result.is_none() && artwork_result.is_none() && json_result.is_none() {
        error!("All processing attempts failed for track {}", track.id);
        cleanup_temp_dir(&work_dir).await?;
        return Err("Failed to process track".into());
    }
    
    info!("Processing completed for track '{}' (ID: {})", track.title, track.id);
    Ok((mp3_result, ogg_result, artwork_result, json_result))
}

/// Transcode a URL to MP3 using ffmpeg
async fn transcode_to_mp3(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing ffmpeg MP3 transcoding command");
    let mut cmd = TokioCommand::new("ffmpeg");
    cmd.arg("-i")
        .arg(url)
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-q:a")
        .arg("2") // High quality (0-9, lower is better)
        .arg("-y") // Overwrite output
        .arg(output_path);
    
    // Log command (without full URL for privacy/security)
    debug!("ffmpeg command: -i [url] -c:a libmp3lame -q:a 2 -y {}", 
          output_path.display());
    
    // Execute command
    let status = cmd.status().await?;
    
    if !status.success() {
        error!("ffmpeg MP3 transcoding failed with exit code: {}", status);
        return Err(format!("ffmpeg failed with exit code: {}", status).into());
    }
    
    debug!("ffmpeg MP3 transcoding completed successfully");
    Ok(())
}

/// Transcode a URL to OGG/Opus using ffmpeg
async fn transcode_to_ogg(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing ffmpeg OGG/Opus transcoding command");
    let mut cmd = TokioCommand::new("ffmpeg");
    cmd.arg("-i")
        .arg(url)
        .arg("-c:a")
        .arg("libopus")
        .arg("-b:a")
        .arg("128k") // Good quality for opus
        .arg("-y") // Overwrite output
        .arg(output_path);
    
    // Log command (without full URL for privacy/security)
    debug!("ffmpeg command: -i [url] -c:a libopus -b:a 128k -y {}", 
          output_path.display());
    
    // Execute command
    let status = cmd.status().await?;
    
    if !status.success() {
        error!("ffmpeg OGG transcoding failed with exit code: {}", status);
        return Err(format!("ffmpeg failed with exit code: {}", status).into());
    }
    
    debug!("ffmpeg OGG transcoding completed successfully");
    Ok(())
}

/// Clean up temporary files after processing
pub async fn cleanup_temp_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if dir.exists() && dir.is_dir() {
        debug!("Cleaning up temporary directory: {}", dir.display());
        match fs::remove_dir_all(dir) {
            Ok(_) => debug!("Successfully removed temp directory: {}", dir.display()),
            Err(e) => warn!("Failed to remove temp directory {}: {}", dir.display(), e),
        }
    } else {
        debug!("Temp directory doesn't exist or is not a directory: {}", dir.display());
    }
    Ok(())
}

/// Delete a temporary file
pub async fn delete_temp_file(path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let p = Path::new(path);
    if p.exists() && p.is_file() {
        debug!("Deleting temporary file: {}", p.display());
        match fs::remove_file(p) {
            Ok(_) => debug!("Successfully deleted temp file: {}", p.display()),
            Err(e) => warn!("Failed to delete temp file {}: {}", p.display(), e),
        }
    } else {
        debug!("File doesn't exist or is not a regular file: {}", p.display());
    }
    Ok(())
}

/// Sanitize a filename to be safe for the file system
fn sanitize_filename(filename: &str) -> String {
    // Replace invalid characters with underscores
    let sanitized = filename
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c
        })
        .collect::<String>();
    
    // Truncate if too long (most filesystems have limits around 255 chars)
    if sanitized.len() > 100 {
        sanitized.chars().take(100).collect()
    } else {
        sanitized
    }
}

/// Check if ffmpeg is available
pub fn check_ffmpeg() -> bool {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(_) => true,
        Err(_) => {
            error!("ffmpeg not found in PATH - audio transcoding will not work");
            false
        }
    }
}

/// Download artwork from URL
async fn download_artwork(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Downloading artwork from URL");
    
    // Create reqwest client
    let client = &HTTP_CLIENT;
    
    // Download the image
    let response = client.get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(format!("Failed to download artwork: HTTP {}", response.status()).into());
    }
    
    // Get the image data
    let image_data = response.bytes().await?;
    
    // Save to file
    let mut file = TokioFile::create(output_path).await?;
    file.write_all(&image_data).await?;
    
    debug!("Artwork downloaded successfully to {}", output_path.display());
    Ok(())
}

/// Save track data as JSON
async fn save_track_json(track: &Track, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Saving track data as JSON to {}", output_path.display());
    
    // Create a serializable structure with all available data
    let mut json_data = serde_json::to_value(track)?;
    
    // Add raw_data if available
    if let Some(raw_data) = &track.raw_data {
        json_data["raw_data"] = raw_data.clone();
    }
    
    // Serialize to pretty JSON
    let json_string = serde_json::to_string_pretty(&json_data)?;
    
    // Save to file
    let mut file = TokioFile::create(output_path).await?;
    file.write_all(json_string.as_bytes()).await?;
    
    debug!("Track data JSON saved successfully");
    Ok(())
}

// Add a lazy_static HTTP client
lazy_static::lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
} 