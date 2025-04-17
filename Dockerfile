FROM rust:1.76-slim-bullseye as builder

# Accept version argument from build command
ARG VERSION=dev

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy Cargo files first to cache dependencies
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code
COPY src/ src/

# Build the application
RUN cargo build --release

FROM debian:bullseye-slim

# Accept version argument from builder stage
ARG VERSION=dev

WORKDIR /app

# Install runtime dependencies (ffmpeg for audio processing)
RUN apt-get update && apt-get install -y \
    ffmpeg \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary from the builder stage
COPY --from=builder /app/target/release/scarchivebot /app/scarchivebot

# Create a directory for temporary files
RUN mkdir -p /app/temp

# Set environment variables
ENV RUST_LOG=info

# Add image labels
LABEL org.opencontainers.image.title="SoundCloud Archiver Bot" \
      org.opencontainers.image.description="Watches SoundCloud users for new tracks and sends them to Discord" \
      org.opencontainers.image.version=${VERSION} \
      org.opencontainers.image.source="https://github.com/SCArchive/scarchivebot" \
      org.opencontainers.image.licenses="MIT"

# Volume mappings for configuration files
VOLUME ["/app/config.json", "/app/users.json", "/app/tracks.json", "/app/temp"]

# Working directory
WORKDIR /app

# Command to run
ENTRYPOINT ["/app/scarchivebot"]

# Help text that shows when running the container with --help
CMD ["--help"] 