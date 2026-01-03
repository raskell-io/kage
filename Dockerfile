# syntax=docker/dockerfile:1.4

# Kage Docker Image
# Uses pre-built binaries for fast multi-arch builds in CI

FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r kage -g 1000 && \
    useradd -r -g kage -u 1000 -m -s /bin/bash kage

# Create required directories
RUN mkdir -p \
    /etc/kage \
    /var/lib/kage \
    /var/log/kage \
    && chown -R kage:kage \
    /etc/kage \
    /var/lib/kage \
    /var/log/kage

# Copy pre-built binary (passed via build context)
COPY kage /usr/local/bin/kage
RUN chmod +x /usr/local/bin/kage

# Switch to non-root user
USER kage

# Environment variables
ENV RUST_LOG=info,kage=info \
    KAGE_DATA_DIR=/var/lib/kage \
    KAGE_LOG_DIR=/var/log/kage

# Expose daemon port
EXPOSE 9876

# Use tini for proper signal handling
ENTRYPOINT ["/usr/bin/tini", "--"]

# Default command - start daemon
CMD ["kage", "daemon", "start", "--foreground"]
