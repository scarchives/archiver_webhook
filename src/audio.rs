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
use serde_json::Value;

/// Download and preserve original audio from a SoundCloud track
/// Returns paths to the downloaded files:
/// - First value: Original stream file (or transcoded MP3 if necessary as fallback)
/// - Second value: Secondary format (like OGG/Opus) if available
/// - Third value: Artwork file
/// - Fourth value: JSON metadata file
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
    
    // Extract all available formats from the raw data
    let available_formats = extract_available_formats(track);
    debug!("Found {} available formats for track {}", available_formats.len(), track.id);
    
    // First try to download all available formats in their original format
    let mut downloaded_files = Vec::new();
    
    // If we have raw transcodings data, use it
    for (format_info, url) in available_formats {
        debug!("Attempting to download format: {} at {}", format_info, url);
        
        // Determine file extension based on format info
        let extension = determine_extension_from_format(&format_info);
        let safe_format = sanitize_format_string(&format_info);
        let output_path = work_dir.join(format!("{}_{}.{}", sanitized_title, safe_format, extension));
        
        debug!("Downloading stream to: {}", output_path.display());
        
        // Use the new resolve_and_download_format function
        match resolve_and_download_format(&format_info, &url, &output_path).await {
            Ok(()) => {
                let file_size = match fs::metadata(&output_path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => 0,
                };
                
                info!("Successfully downloaded {} format: {} ({} bytes)", 
                      format_info, output_path.display(), file_size);
                downloaded_files.push((format_info, output_path.to_string_lossy().to_string()));
            },
            Err(e) => {
                warn!("Failed to download {} format: {}", format_info, e);
                // Continue to next format
            }
        }
    }
    
    // Fallback: Use our existing HLS and stream_url fields if we didn't get anything
    if downloaded_files.is_empty() {
        debug!("No formats downloaded from transcodings, falling back to HLS/stream URLs");
        
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
        let stream_url = if downloaded_files.is_empty() {
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
        
        // Try to download from HLS URL
        if let Some(url) = &hls_url {
            let output_path = work_dir.join(format!("{}_hls.m4a", sanitized_title));
            debug!("Downloading HLS stream to: {}", output_path.display());
            
            match download_stream(url, &output_path).await {
                Ok(()) => {
                    let file_size = match fs::metadata(&output_path) {
                        Ok(metadata) => metadata.len(),
                        Err(_) => 0,
                    };
                    
                    info!("Successfully downloaded HLS stream: {} ({} bytes)", 
                         output_path.display(), file_size);
                    downloaded_files.push(("hls/aac".to_string(), output_path.to_string_lossy().to_string()));
                },
                Err(e) => {
                    warn!("Failed to download HLS stream: {}", e);
                    
                    // If we failed to download directly, use ffmpeg with stream copy as fallback
                    info!("Trying ffmpeg with stream copy for HLS URL");
                    match ffmpeg_stream_copy(url, &output_path).await {
                        Ok(()) => {
                            let file_size = match fs::metadata(&output_path) {
                                Ok(metadata) => metadata.len(),
                                Err(_) => 0,
                            };
                            
                            info!("Successfully saved HLS stream with ffmpeg: {} ({} bytes)", 
                                 output_path.display(), file_size);
                            downloaded_files.push(("hls/aac".to_string(), output_path.to_string_lossy().to_string()));
                        },
                        Err(e) => {
                            warn!("Failed to save HLS stream with ffmpeg: {}", e);
                        }
                    }
                }
            }
        }
        
        // Try to download from stream URL if we don't have anything yet
        if downloaded_files.is_empty() && stream_url.is_some() {
            let url = stream_url.as_ref().unwrap();
            let output_path = work_dir.join(format!("{}_stream.mp3", sanitized_title));
            debug!("Downloading progressive stream to: {}", output_path.display());
            
            match download_stream(url, &output_path).await {
                Ok(()) => {
                    let file_size = match fs::metadata(&output_path) {
                        Ok(metadata) => metadata.len(),
                        Err(_) => 0,
                    };
                    
                    info!("Successfully downloaded progressive stream: {} ({} bytes)", 
                         output_path.display(), file_size);
                    downloaded_files.push(("progressive/mp3".to_string(), output_path.to_string_lossy().to_string()));
                },
                Err(e) => {
                    warn!("Failed to download progressive stream: {}", e);
                    
                    // If we failed to download directly, use ffmpeg with stream copy as fallback
                    info!("Trying ffmpeg with stream copy for progressive URL");
                    match ffmpeg_stream_copy(url, &output_path).await {
                        Ok(()) => {
                            let file_size = match fs::metadata(&output_path) {
                                Ok(metadata) => metadata.len(),
                                Err(_) => 0,
                            };
                            
                            info!("Successfully saved progressive stream with ffmpeg: {} ({} bytes)", 
                                 output_path.display(), file_size);
                            downloaded_files.push(("progressive/mp3".to_string(), output_path.to_string_lossy().to_string()));
                        },
                        Err(e) => {
                            warn!("Failed to save progressive stream with ffmpeg: {}", e);
                        }
                    }
                }
            }
        }
        
        // Last resort: If we still have nothing, try transcoding as before
        if downloaded_files.is_empty() && (hls_url.is_some() || stream_url.is_some()) {
            warn!("Direct downloads failed, falling back to transcoding");
            
            // File paths for transcoded files
            let mp3_path = work_dir.join(format!("{}.mp3", sanitized_title));
            
            if let Some(url) = hls_url.as_ref().or(stream_url.as_ref()) {
                info!("Starting MP3 transcoding for track {} (fallback mode)", track.id);
                debug!("Input URL: {}", url);
                debug!("Output path: {}", mp3_path.display());
                
                match transcode_to_mp3(url, &mp3_path).await {
                    Ok(_) => {
                        let file_size = match fs::metadata(&mp3_path) {
                            Ok(metadata) => metadata.len(),
                            Err(_) => 0,
                        };
                        
                        info!("Successfully transcoded to MP3 (fallback): {} ({} bytes)", 
                             mp3_path.display(), file_size);
                        downloaded_files.push(("transcoded/mp3".to_string(), mp3_path.to_string_lossy().to_string()));
                    },
                    Err(e) => {
                        error!("Failed to transcode to MP3 (fallback): {}", e);
                    }
                }
            }
        }
    }
    
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
    
    // If we have no audio files, return error
    if downloaded_files.is_empty() && json_result.is_none() && artwork_result.is_none() {
        error!("No valid audio URLs or data found for track {}", track.id);
        cleanup_temp_dir(&work_dir).await?;
        return Err("No valid audio URLs or data found for track".into());
    }
    
    // Sort files by preference for primary/secondary output
    downloaded_files.sort_by(|(format_a, _), (format_b, _)| {
        // Prioritize formats based on quality/preference
        let priority_a = get_format_priority(format_a);
        let priority_b = get_format_priority(format_b);
        priority_a.cmp(&priority_b)
    });
    
    // Return the primary and secondary files if available
    let primary_file = downloaded_files.get(0).map(|(_, path)| path.clone());
    let secondary_file = downloaded_files.get(1).map(|(_, path)| path.clone());
    
    info!("Processing completed for track '{}' (ID: {})", track.title, track.id);
    debug!("Primary file: {:?}, Secondary file: {:?}", primary_file, secondary_file);
    
    Ok((primary_file, secondary_file, artwork_result, json_result))
}

/// Extract all available streaming formats from track data
fn extract_available_formats(track: &Track) -> Vec<(String, String)> {
    let mut formats = Vec::new();
    
    if let Some(raw_data) = &track.raw_data {
        if let Some(media) = raw_data.get("media") {
            if let Some(transcodings) = media.get("transcodings").and_then(Value::as_array) {
                debug!("Found {} total transcodings for track {}", transcodings.len(), track.id);
                
                for transcoding in transcodings {
                    // Get format info
                    let mime_type = transcoding.get("format")
                        .and_then(|f| f.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    
                    let protocol = transcoding.get("format")
                        .and_then(|f| f.get("protocol"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    
                    let quality = transcoding.get("quality")
                        .and_then(Value::as_str)
                        .unwrap_or("sq");
                    
                    // Format string with additional info to help debugging
                    let format_string = format!("{}/{}/{}", protocol, mime_type, quality);
                    
                    // Skip certain formats that are known to cause issues
                    if format_string.contains("audio/mpegurl") && protocol == "hls" {
                        // This is an old/deprecated format specification that often 404s
                        debug!("Skipping known problematic format: {}", format_string);
                        continue;
                    }
                    
                    // Get URL
                    if let Some(url) = transcoding.get("url").and_then(Value::as_str) {
                        debug!("Found format: {} at URL: {}", format_string, url);
                        formats.push((format_string, url.to_string()));
                    }
                }
            }
        }
    }
    
    // Sort formats by priority, so we try the best ones first
    formats.sort_by(|(format_a, _), (format_b, _)| {
        let priority_a = get_format_priority(format_a);
        let priority_b = get_format_priority(format_b);
        priority_a.cmp(&priority_b)
    });
    
    debug!("Sorted formats by priority: {}", 
           formats.iter()
                 .map(|(fmt, _)| fmt.as_str())
                 .collect::<Vec<&str>>()
                 .join(", "));
    
    formats
}

/// Determine file extension based on format info
fn determine_extension_from_format(format_info: &str) -> String {
    if format_info.contains("audio/mpeg") {
        "mp3".to_string()
    } else if format_info.contains("audio/ogg") {
        if format_info.contains("codecs=\"opus\"") {
            "opus".to_string()
        } else {
            "ogg".to_string()
        }
    } else if format_info.contains("audio/mp4") || format_info.contains("aac") {
        "m4a".to_string()
    } else if format_info.contains("audio/x-wav") {
        "wav".to_string()
    } else if format_info.contains("flac") {
        "flac".to_string()
    } else if format_info.contains("hls") {
        "m4a".to_string()  // HLS usually contains AAC in MP4 container
    } else {
        // Default fallback
        "audio".to_string()
    }
}

/// Sanitize format string for use in filenames
fn sanitize_format_string(format_info: &str) -> String {
    // Replace characters that are problematic in filenames
    let sanitized = format_info
        .replace("/", "_")
        .replace("\\", "_")
        .replace(":", "-")
        .replace("*", "")
        .replace("?", "")
        .replace("\"", "")
        .replace("<", "")
        .replace(">", "")
        .replace("|", "_")
        .replace(";", "_")
        .replace("=", "_")
        .replace(",", "_")
        .replace(" ", "_")
        .replace("codecs=", "")
        .replace("mp4a.40.2", "aac")
        .replace(".", "_");
    
    // Truncate if too long
    if sanitized.len() > 50 {
        sanitized.chars().take(50).collect()
    } else {
        sanitized
    }
}

/// Get priority for format sorting (lower number = higher priority)
fn get_format_priority(format_info: &str) -> i32 {
    if format_info.contains("hq") {
        // High quality gets priority
        if format_info.contains("flac") {
            1  // FLAC HQ (rare)
        } else if format_info.contains("opus") {
            2  // Opus HQ
        } else if format_info.contains("mp3") {
            3  // MP3 HQ
        } else if format_info.contains("aac") || format_info.contains("mp4") {
            4  // AAC HQ
        } else {
            5  // Other HQ
        }
    } else {
        // Regular quality
        if format_info.contains("progressive") && format_info.contains("mp3") {
            10  // Progressive MP3 (common format)
        } else if format_info.contains("opus") {
            11  // Opus standard quality
        } else if format_info.contains("mp3") {
            12  // MP3 standard quality
        } else if format_info.contains("aac") || format_info.contains("mp4") {
            13  // AAC standard quality
        } else if format_info.contains("hls") {
            15  // HLS (can contain various formats, often AAC)
        } else if format_info.contains("transcoded") {
            50  // Fallback transcoded files (lowest priority)
        } else {
            20  // Other formats
        }
    }
}

/// Download a stream directly
async fn download_stream(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // For streaming URLs, direct downloads often produce incomplete files
    // Instead, always use ffmpeg to properly download and process streams
    debug!("Using ffmpeg to download stream from {}", url);
    
    // Use ffmpeg with stream copy to preserve original quality
    ffmpeg_stream_copy(url, output_path).await
}

/// Use ffmpeg to copy the stream without transcoding
async fn ffmpeg_stream_copy(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing ffmpeg stream copy command");
    let mut cmd = TokioCommand::new("ffmpeg");
    
    // Check if we should show ffmpeg output
    let show_output = match crate::config::Config::show_ffmpeg_output() {
        Some(true) => true,
        _ => false,
    };
    
    cmd.arg("-i")
        .arg(url)
        .arg("-c")
        .arg("copy")  // Copy the stream without re-encoding
        .arg("-y");  // Overwrite output
    
    // Configure stdout/stderr redirection based on config
    if !show_output {
        // Silence ffmpeg output
        cmd.stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
    }
    
    // Add output path
    cmd.arg(output_path);
    
    // Log command (without full URL for privacy/security)
    debug!("ffmpeg command: -i [url] -c copy -y {}", 
          output_path.display());
    
    // Execute command
    let status = cmd.status().await?;
    
    if !status.success() {
        error!("ffmpeg stream copy failed with exit code: {}", status);
        
        // If stream copy fails, try again with default codec selection
        // This is necessary for some formats (especially HLS) where stream copy might not work
        debug!("Retrying with default codec selection");
        
        let mut cmd2 = TokioCommand::new("ffmpeg");
        cmd2.arg("-i")
            .arg(url)
            .arg("-y");  // Overwrite output
        
        // Configure stdout/stderr redirection based on config
        if !show_output {
            // Silence ffmpeg output
            cmd2.stdout(std::process::Stdio::null())
               .stderr(std::process::Stdio::null());
        }
        
        // Add output path
        cmd2.arg(output_path);
        
        debug!("ffmpeg retry command: -i [url] -y {}", output_path.display());
        
        let retry_status = cmd2.status().await?;
        
        if !retry_status.success() {
            error!("ffmpeg retry failed with exit code: {}", retry_status);
            return Err(format!("ffmpeg failed with exit code: {}", retry_status).into());
        }
    }
    
    debug!("ffmpeg stream copy completed successfully");
    Ok(())
}

/// Transcode a URL to MP3 using ffmpeg (fallback method)
async fn transcode_to_mp3(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing ffmpeg MP3 transcoding command");
    let mut cmd = TokioCommand::new("ffmpeg");
    
    // Check if we should show ffmpeg output
    let show_output = match crate::config::Config::show_ffmpeg_output() {
        Some(true) => true,
        _ => false,
    };
    
    cmd.arg("-i")
        .arg(url)
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-q:a")
        .arg("2") // High quality (0-9, lower is better)
        .arg("-y"); // Overwrite output
    
    // Configure stdout/stderr redirection based on config
    if !show_output {
        // Silence ffmpeg output
        cmd.stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
    }
    
    // Add output path
    cmd.arg(output_path);
    
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

/// Resolve the stream URL
async fn resolve_and_download_format(
    format_info: &str, 
    url: &str, 
    output_path: &Path
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Resolving and downloading format: {}", format_info);
    
    match get_stream_url(url).await {
        Ok(resolved_url) => {
            // Download the stream
            match download_stream(&resolved_url, output_path).await {
                Ok(()) => {
                    // Check if file is large enough to be a valid audio file
                    let file_size = match fs::metadata(output_path) {
                        Ok(metadata) => metadata.len(),
                        Err(_) => 0,
                    };
                    
                    if file_size < 1024 { // Less than 1KB is suspicious
                        return Err(format!("Downloaded file too small ({} bytes)", file_size).into());
                    }
                    
                    debug!("Successfully downloaded {} format: {} bytes", format_info, file_size);
                    Ok(())
                },
                Err(e) => Err(e)
            }
        },
        Err(e) => {
            // Check for specific error types to handle appropriately
            let err_string = e.to_string();
            
            if err_string.contains("HTTP error 401") || err_string.contains("HTTP error 403") {
                // Authentication errors - this is likely a premium-only format
                warn!("Format {} requires authentication (premium only)", format_info);
                return Err(format!("Authentication required for {}", format_info).into());
            } else if err_string.contains("HTTP error 404") {
                // Resource not found
                warn!("Format {} returned 404 Not Found", format_info);
                return Err(format!("Resource not found for {}", format_info).into());
            }
            
            // General error
            Err(e)
        }
    }
}

// Add a lazy_static HTTP client
lazy_static::lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
} 