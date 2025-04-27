# SoundCloud Archiver Webhook

A Rust application that watches SoundCloud users for new tracks and sends them to a Discord webhook with rich formatting and audio files.

## Features

- Monitors SoundCloud users for new track uploads
- Downloads all available audio formats (MP3, AAC, Opus, etc.) for best quality preservation
- Downloads original high-resolution artwork
- Creates complete JSON snapshots of track metadata
- Sends rich embeds to Discord with track details and media files
- Simple tracks database for persistent state tracking
- Configurable polling interval
- Automatic client ID regeneration
- Optional scraping of users' liked tracks
- Auto-follow mode to automatically add new followings from a source user
- Parallel processing of tracks and transcoding operations

## Requirements

### Standard Installation
- Rust 1.70+
- `ffmpeg` command line utility must be in your PATH for audio transcoding

### Docker Installation
- Docker
- Docker Compose (optional)

## Setup

### Standard Installation

1. Clone the repository
2. Build with Cargo:
   ```bash
   cargo build --release
   ```
3. Create a `config.json` file in the same directory as the executable:
   ```json
   {
     "discord_webhook_url": "YOUR_DISCORD_WEBHOOK_URL",
     "log_level": "info",
     "poll_interval_sec": 60,
     "users_file": "users.json",
     "tracks_file": "tracks.json",
     "max_tracks_per_user": 500,
     "pagination_size": 50,
     "temp_dir": null,
     "max_parallel_fetches": 4,
     "max_concurrent_processing": 2,
     "scrape_user_likes": false,
     "max_likes_per_user": 500,
     "auto_follow_source": null,
     "auto_follow_interval": 24,
     "db_save_interval": 1,
     "db_save_tracks": 5,
     "show_ffmpeg_output": false,
     "log_file": "latest.log"
   }
   ```
4. Create a `users.json` file with the SoundCloud user IDs to watch:
   ```json
   {
     "users": [
       "123456",
       "789012"
     ]
   }
   ```

### Docker Installation

1. Make sure Docker is installed on your system
2. Create the required configuration files in your project directory:
   - `config.json` (same format as above, but set `"temp_dir": "/app/temp"`)
   - `users.json` (same format as above)
   - Create an empty `tracks.json` file or let the application create it
3. Create a `temp` directory for temporary files:
   ```bash
   mkdir -p temp
   ```

#### Using Pre-built Image

```bash
# Pull the latest image
docker pull ghcr.io/scarchive/archiver_webhook:latest

# Run with your configuration files
docker run -d --name archiver_webhook \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  ghcr.io/scarchive/archiver_webhook:latest
```

#### Using Docker Compose (recommended)

Create a `docker-compose.yml` file:

```yaml
version: '3'

services:
  archiver_webhook:
    image: ghcr.io/scarchive/archiver_webhook:latest
    container_name: archiver_webhook
    restart: unless-stopped
    volumes:
      - ./config.json:/app/config.json:ro
      - ./users.json:/app/users.json:rw
      - ./tracks.json:/app/tracks.json:rw
      - ./temp:/app/temp:rw
    command: ""
```

Then run:

```bash
docker-compose up -d
```

View logs:

```bash
docker-compose logs -f
```

Run one-time commands:

```bash
# Resolve a SoundCloud URL
docker-compose run --rm archiver_webhook --resolve https://soundcloud.com/artist/track-name

# Initialize tracks database
docker-compose run --rm archiver_webhook --init-tracks

# Post a specific track
docker-compose run --rm archiver_webhook --post-track 1234567890
```

#### Building the Image Locally

Build the image:

```bash
docker build -t archiver_webhook .
```

Run in watcher mode:

```bash
docker run -d --name archiver_webhook \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  archiver_webhook
```

Run one-time commands:

```bash
docker run --rm \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  archiver_webhook --resolve https://soundcloud.com/artist/track-name
```

## Configuration Options

- `discord_webhook_url` (required): The Discord webhook URL to send track notifications to
- `log_level` (default: "info"): Logging level for the application
- `poll_interval_sec` (default: 60): How often to check for new tracks, in seconds
- `users_file` (default: "users.json"): Path to the file containing user IDs to watch
- `tracks_file` (default: "tracks.json"): Path to the tracks database file for persistent storage
- `max_tracks_per_user` (default: 500): Maximum number of tracks to fetch per user (total limit)
- `pagination_size` (default: 50): Number of tracks/likes to fetch per API request (pagination size)
- `temp_dir` (optional): Directory for temporary files (if not specified, system temp dir is used)
- `max_parallel_fetches` (default: 4): Maximum number of users to process in parallel
- `max_concurrent_processing` (default: 2): Maximum number of concurrent ffmpeg processes per user
- `scrape_user_likes` (default: false): Whether to scrape liked tracks from users being monitored
- `max_likes_per_user` (default: 500): Maximum number of likes to fetch for each user when `scrape_user_likes` is enabled (uses `pagination_size` for API requests)
- `auto_follow_source` (optional): User ID or URL whose followings you want to automatically add to your watched users
- `auto_follow_interval` (default: 24): How often to check for new followings (in poll cycles). Checking is also performed once immediately on startup.
- `db_save_interval` (default: 1): How often to save the database (in poll cycles).
- `db_save_tracks` (default: 5): Number of new tracks to process before automatically saving the database. This works in addition to the time-based saving with `db_save_interval`.
- `show_ffmpeg_output` (default: false): Whether to show ffmpeg output in the console logs
- `log_file` (default: "latest.log"): Path to the log file for application logs

## Usage

### Standard Installation

Run the application in watcher mode (default):

```bash
./archiver_webhook
```

To resolve a SoundCloud URL and get information (artist, track, user info):

```bash
./archiver_webhook --resolve https://soundcloud.com/artist/track-name
```

To initialize the tracks database with all existing tracks from watched users:

```bash
./archiver_webhook --init-tracks
```

To post a specific track to Discord without adding it to the database:

```bash
./archiver_webhook --post-track 1234567890
# Or with a URL
./archiver_webhook --post-track https://soundcloud.com/artist/track-name
```

To interactively generate config.json and users.json based on a SoundCloud user's followings:

```bash
./archiver_webhook --generate-config https://soundcloud.com/user-to-follow
```

This will:
1. Fetch the user's profile
2. Get all users they follow
3. Interactively create config.json with default values
4. Generate users.json with all followed users' IDs
5. Display track counts for each user for reference

Defaults can be accepted by pressing Enter for each prompt.

# Logging

Logging is controlled by the `log_level` field in your `config.json`.
Valid values: `trace`, `debug`, `info`, `warn`, `error` (default: `info`).

The application provides:
- Console output with colored log levels
- File logging to `latest.log` (or custom path specified in config)
- Windows-specific console title updates showing current stats:
  ```
  SCArchive Webhook | Tracks: 123456 | New: 500 | Errors: 14
  ```

Example config:
```json
{
  "log_level": "debug",
  "log_file": "latest.log",
  ...
}
```

## How to Find SoundCloud User IDs

SoundCloud doesn't expose user IDs directly in the UI, but you can find them by:

1. Going to the user's profile page
2. Using the `--resolve` command with the user's profile URL
3. The command will display the user ID which you can then add to your users.json file

## What Gets Archived

For each track, the bot will:
1. Download all available audio formats (MP3, AAC, Opus, etc.) depending on what SoundCloud provides
2. Download the original high-resolution artwork
3. Create a complete JSON snapshot of all track metadata
4. Send everything to Discord with a rich embed containing track details
5. Automatically handle Discord's upload restrictions (8MB per file limit, max 10 attachments per message)

The bot attempts to preserve all available audio qualities and formats rather than just converting to MP3/OGG.

## Limitations

- Discord has attachment size limits (8MB per file for regular servers, 50MB per file for Nitro-boosted servers)
- Rate limits apply to both SoundCloud API and Discord webhooks
- FFMPEG must be installed and in PATH for audio transcoding

## License

This project is licensed under the MIT License - see the LICENSE file for details. 
