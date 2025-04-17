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
        Some(dir) => PathBuf::from(dir),
        None => env::temp_dir(),
    };
    
    // Create a unique subfolder for this download
    let unique_id = Uuid::new_v4().to_string();
    let work_dir = base_dir.join(format!("scarchive_{}", unique_id));
    fs::create_dir_all(&work_dir)?;
    
    info!("Processing audio for track {} in {}", track.id, work_dir.display());
    
    // Resolve the HLS URL if we have one
    let hls_url = match &track.hls_url {
        Some(url) => {
            let resolved = get_stream_url(url).await?;
            info!("Resolved HLS URL for track {}", track.id);
            Some(resolved)
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
                let resolved = get_stream_url(url).await?;
                info!("Resolved stream URL for track {}", track.id);
                Some(resolved)
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
        cleanup_temp_dir(&work_dir).await?;
        return Err("No valid audio URLs found for track".into());
    }
    
    // File paths for transcoded files
    let mp3_path = work_dir.join(format!("{}.mp3", sanitize_filename(&track.title)));
    let ogg_path = work_dir.join(format!("{}.ogg", sanitize_filename(&track.title)));
    
    // Try to transcode to both formats
    let mut mp3_result = None;
    let mut ogg_result = None;
    
    // Process MP3 (preferring HLS if available)
    if let Some(url) = hls_url.as_ref().or(stream_url.as_ref()) {
        debug!("Transcoding to MP3: {}", mp3_path.display());
        if transcode_to_mp3(url, &mp3_path).await.is_ok() {
            mp3_result = Some(mp3_path.to_string_lossy().to_string());
            info!("Successfully transcoded to MP3: {}", mp3_path.display());
        } else {
            warn!("Failed to transcode to MP3");
        }
    }
    
    // Process OGG (only from HLS for better quality)
    if let Some(url) = &hls_url {
        debug!("Transcoding to OGG: {}", ogg_path.display());
        if transcode_to_ogg(url, &ogg_path).await.is_ok() {
            ogg_result = Some(ogg_path.to_string_lossy().to_string());
            info!("Successfully transcoded to OGG: {}", ogg_path.display());
        } else {
            warn!("Failed to transcode to OGG");
        }
    }
    
    // If both failed, return error
    if mp3_result.is_none() && ogg_result.is_none() {
        cleanup_temp_dir(&work_dir).await?;
        return Err("Failed to transcode track to any format".into());
    }
    
    Ok((mp3_result, ogg_result))
}

/// Transcode a URL to MP3 using ffmpeg
async fn transcode_to_mp3(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let status = TokioCommand::new("ffmpeg")
        .arg("-i")
        .arg(url)
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-q:a")
        .arg("2") // High quality (0-9, lower is better)
        .arg("-y") // Overwrite output
        .arg(output_path)
        .status()
        .await?;
    
    if !status.success() {
        return Err(format!("ffmpeg failed with exit code: {}", status).into());
    }
    
    Ok(())
}

/// Transcode a URL to OGG/Opus using ffmpeg
async fn transcode_to_ogg(url: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let status = TokioCommand::new("ffmpeg")
        .arg("-i")
        .arg(url)
        .arg("-c:a")
        .arg("libopus")
        .arg("-b:a")
        .arg("128k") // Good quality for opus
        .arg("-y") // Overwrite output
        .arg(output_path)
        .status()
        .await?;
    
    if !status.success() {
        return Err(format!("ffmpeg failed with exit code: {}", status).into());
    }
    
    Ok(())
}

/// Clean up temporary files after processing
pub async fn cleanup_temp_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dir.exists() && dir.is_dir() {
        debug!("Cleaning up temporary directory: {}", dir.display());
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Delete a temporary file
pub async fn delete_temp_file(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let p = Path::new(path);
    if p.exists() && p.is_file() {
        debug!("Deleting temporary file: {}", p.display());
        fs::remove_file(p)?;
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