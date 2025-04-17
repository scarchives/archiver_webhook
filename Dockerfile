FROM rust:1.86-alpine AS builder

# Install build dependencies
RUN apk add --no-cache build-base pkgconfig openssl-dev

WORKDIR /app

# Copy everything needed for the build
COPY . .

# Build the application
RUN cargo build --release

FROM alpine:3.21

# Install runtime dependencies
RUN apk add --no-cache ffmpeg ca-certificates && \
    mkdir -p /app/temp

WORKDIR /app

# Copy only the compiled binary from builder
COPY --from=builder /app/target/release/scarchivebot /app/scarchivebot

# Set environment variables
ENV RUST_LOG=info

# Add image labels
LABEL org.opencontainers.image.title="SoundCloud Archiver Bot" \
      org.opencontainers.image.description="Watches SoundCloud users for new tracks and sends them to Discord" \
      org.opencontainers.image.source="https://github.com/scarchive/scarchivebot" \
      org.opencontainers.image.licenses="MIT"

# Run the application
ENTRYPOINT ["/app/scarchivebot"]
CMD ["--help"] 