# SoundCloud Archiver Bot

A Rust application that watches SoundCloud users for new tracks and sends them to a Discord webhook with rich formatting and audio files.

## Features

- Monitors SoundCloud users for new track uploads
- Downloads and transcodes tracks to MP3 and OGG formats
- Downloads original high-resolution artwork
- Creates complete JSON snapshots of track metadata
- Sends rich embeds to Discord with track details and media files
- Simple tracks database for persistent state tracking
- Configurable polling interval
- Automatic client ID regeneration

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
     "poll_interval_sec": 60,
     "users_file": "users.json",
     "tracks_file": "tracks.json",
     "max_tracks_per_user": 200,
     "temp_dir": null
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

## Configuration Options

- `discord_webhook_url` (required): The Discord webhook URL to send track notifications to
- `poll_interval_sec` (default: 60): How often to check for new tracks, in seconds
- `users_file` (default: "users.json"): Path to the file containing user IDs to watch
- `tracks_file` (default: "tracks.json"): Path to the tracks database file for persistent storage
- `max_tracks_per_user` (default: 200): Maximum number of tracks to fetch per user
- `temp_dir` (optional): Directory for temporary files (if not specified, system temp dir is used)

## Usage

### Standard Installation

Run the application in watcher mode (default):

```bash
./scarchivebot
```

To resolve a SoundCloud URL and get information (artist, track, user info):

```bash
./scarchivebot --resolve https://soundcloud.com/artist/track-name
```

To initialize the tracks database with all existing tracks from watched users:

```bash
./scarchivebot --init-tracks
```

To post a specific track to Discord without adding it to the database:

```bash
./scarchivebot --post-track 1234567890
# Or with a URL
./scarchivebot --post-track https://soundcloud.com/artist/track-name
```

Set the `RUST_LOG` environment variable to control logging level:

```bash
RUST_LOG=info ./scarchivebot
```

Valid log levels: `trace`, `debug`, `info`, `warn`, `error`

### Docker Installation

#### Using Pre-built Image

You can use the pre-built Docker image from GitHub Container Registry:

```bash
# Pull the latest image
docker pull ghcr.io/SCArchive/scarchivebot:latest

# Run with your configuration files
docker run -d --name scarchivebot \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  -e RUST_LOG=info \
  ghcr.io/SCArchive/scarchivebot:latest
```

#### Using Docker Compose (recommended)

Create a `docker-compose.yml` file:

```yaml
version: '3'

services:
  scarchivebot:
    image: ghcr.io/SCArchive/scarchivebot:latest
    container_name: scarchivebot
    restart: unless-stopped
    volumes:
      - ./config.json:/app/config.json:ro
      - ./users.json:/app/users.json:rw
      - ./tracks.json:/app/tracks.json:rw
      - ./temp:/app/temp:rw
    environment:
      - RUST_LOG=${RUST_LOG:-info}
    # By default, run in watcher mode
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
docker-compose run --rm scarchivebot --resolve https://soundcloud.com/artist/track-name

# Initialize tracks database
docker-compose run --rm scarchivebot --init-tracks

# Post a specific track
docker-compose run --rm scarchivebot --post-track 1234567890
```

#### Building the Image Locally

Build the image:

```bash
docker build -t scarchivebot .
```

Run in watcher mode:

```bash
docker run -d --name scarchivebot \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  -e RUST_LOG=info \
  scarchivebot
```

Run one-time commands:

```bash
docker run --rm \
  -v "$(pwd)/config.json:/app/config.json:ro" \
  -v "$(pwd)/users.json:/app/users.json:rw" \
  -v "$(pwd)/tracks.json:/app/tracks.json:rw" \
  -v "$(pwd)/temp:/app/temp:rw" \
  scarchivebot --resolve https://soundcloud.com/artist/track-name
```

## How to Find SoundCloud User IDs

SoundCloud doesn't expose user IDs directly in the UI, but you can find them by:

1. Going to the user's profile page
2. Using the `--resolve` command with the user's profile URL
3. The command will display the user ID which you can then add to your users.json file

## What Gets Archived

For each track, the bot will:
1. Download and transcode audio to MP3 and OGG formats
2. Download the original high-resolution artwork
3. Create a complete JSON snapshot of all track metadata
4. Send everything to Discord with a rich embed containing track details

## Limitations

- Discord has attachment size limits (8MB for regular, 50MB for Nitro boosts)
- Rate limits apply to both SoundCloud API and Discord webhooks
- FFMPEG must be installed and in PATH for audio transcoding

## License

This project is licensed under the MIT License - see the LICENSE file for details. 