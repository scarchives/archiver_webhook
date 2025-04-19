FROM rust:1.86-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y build-essential pkg-config libssl-dev

WORKDIR /app

# Copy everything needed for the build
COPY . .

# Build the application
RUN cargo build --release

FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y ffmpeg ca-certificates && \
    mkdir -p /app/temp

WORKDIR /app

# Copy only the compiled binary from builder
COPY --from=builder /app/target/release/scraper_webhook /app/scraper_webhook

# Set environment variables
ENV RUST_LOG=info

# Add image labels
LABEL org.opencontainers.image.title="SoundCloud Scraper Webhook" \
      org.opencontainers.image.description="Watches SoundCloud users for new tracks and sends them to Discord" \
      org.opencontainers.image.source="https://github.com/scarchive/scraper_webhook" \
      org.opencontainers.image.licenses="MIT"

# Run the application
ENTRYPOINT ["/app/scraper_webhook"]