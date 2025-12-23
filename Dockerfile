# Local development build
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY src ./src

RUN cargo build --release
RUN cp target/release/vibemq /vibemq

# Final minimal image
FROM scratch
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /vibemq /vibemq
COPY vibemq.toml /etc/vibemq/config.toml

EXPOSE 1883 9001
ENTRYPOINT ["/vibemq"]
CMD ["--config", "/etc/vibemq/config.toml"]
