# SoundCloud Archiver Bot

A Rust application that watches SoundCloud users for new tracks and sends them to a Discord webhook with rich formatting and audio files.

## Features

- Monitors SoundCloud users for new track uploads
- Downloads and transcodes tracks to MP3 and OGG formats
- Sends rich embeds to Discord with track details and audio files
- Ephemeral or persistent state tracking
- Configurable polling interval
- Automatic client ID regeneration

## Requirements

- Rust 1.70+
- `ffmpeg` command line utility must be in your PATH for audio transcoding

## Setup

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
     "db_file": "db.json",
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

## Configuration Options

- `discord_webhook_url` (required): The Discord webhook URL to send track notifications to
- `poll_interval_sec` (default: 60): How often to check for new tracks, in seconds
- `users_file` (default: "users.json"): Path to the file containing user IDs to watch
- `db_file` (optional): Path to the database file for persistent storage (if not specified, in-memory DB is used)
- `max_tracks_per_user` (default: 200): Maximum number of tracks to fetch per user
- `temp_dir` (optional): Directory for temporary files (if not specified, system temp dir is used)

## Usage

Run the application in watcher mode (default):

```bash
./scarchivebot
```

To resolve a SoundCloud URL and get information (artist, track, user info):

```bash
./scarchivebot --resolve https://soundcloud.com/artist/track-name
```

This is useful for finding user IDs to add to your watchlist.

Set the `RUST_LOG` environment variable to control logging level:

```bash
RUST_LOG=info ./scarchivebot
```

Valid log levels: `trace`, `debug`, `info`, `warn`, `error`

## How to Find SoundCloud User IDs

SoundCloud doesn't expose user IDs directly in the UI, but you can find them by:

1. Going to the user's profile page
2. Right-clicking and viewing the page source
3. Searching for `"id":` followed by a number (e.g., `"id":123456`)

## Limitations

- Discord has attachment size limits (8MB for regular, 50MB for Nitro boosts)
- Rate limits apply to both SoundCloud API and Discord webhooks
- FFMPEG must be installed and in PATH for audio transcoding

## License

This project is licensed under the MIT License - see the LICENSE file for details. 