use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::env;
use log::{info, warn, error, debug};
use tokio::process::Command as TokioCommand;
use uuid::Uuid;
use crate::soundcloud::{Track, get_stream_url};

/// Download and transcode audio from a SoundCloud track
/// Returns paths to the MP3 and OGG files
pub async fn process_track_audio(
    track: &Track,
    temp_dir: Option<&str>
) -> Result<(Option<String>, Option<String>), Box<dyn std::error::Error>> {
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
    
    // If we have neither, return error
    if hls_url.is_none() && stream_url.is_none() {
        error!("No valid audio URLs found for track {}", track.id);
        cleanup_temp_dir(&work_dir).await?;
        return Err("No valid audio URLs found for track".into());
    }
    
    // File paths for transcoded files
    let sanitized_title = sanitize_filename(&track.title);
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
    
    // If both failed, return error
    if mp3_result.is_none() && ogg_result.is_none() {
        error!("All transcoding attempts failed for track {}", track.id);
        cleanup_temp_dir(&work_dir).await?;
        return Err("Failed to transcode track to any format".into());
    }
    
    info!("Audio processing completed for track '{}' (ID: {})", track.title, track.id);
    Ok((mp3_result, ogg_result))
}

/// Transcode a URL to MP3 using ffmpeg
async fn transcode_to_mp3(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
async fn transcode_to_ogg(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
pub async fn cleanup_temp_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
pub async fn delete_temp_file(path: &str) -> Result<(), Box<dyn std::error::Error>> {
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