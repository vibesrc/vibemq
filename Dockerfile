FROM rust:1.91-alpine AS builder

# Install MUSL compiler + build tools
RUN apk add --no-cache musl-dev build-base

# Get platform from buildx
ARG TARGETPLATFORM

# Determine correct Rust target triple
RUN case "$TARGETPLATFORM" in \
    "linux/amd64")   echo "x86_64-unknown-linux-musl"        > /rust_target ;; \
    "linux/arm64")   echo "aarch64-unknown-linux-musl"       > /rust_target ;; \
    "linux/arm/v7")  echo "armv7-unknown-linux-musleabihf"   > /rust_target ;; \
    *) echo "Unsupported platform: $TARGETPLATFORM" && exit 1 ;; \
    esac

# Add the rust target
RUN rustup target add $(cat /rust_target)

WORKDIR /app

# Copy manifests first for caching
COPY Cargo.toml Cargo.lock ./

# Dummy src for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (cached)
RUN cargo build --release --target $(cat /rust_target)

# Remove dummy
RUN rm -rf src

# Copy actual source code
COPY src ./src

# Build final static binary
RUN cargo build --release --target $(cat /rust_target)
RUN cp target/$(cat /rust_target)/release/vibemq /vibemq

FROM scratch

COPY --from=builder /vibemq /vibemq

EXPOSE 1883 9001
ENTRYPOINT ["/vibemq"]
CMD ["--config", "/etc/vibemq/config.toml"]
