FROM rust:1.92-trixie AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml

RUN mkdir -p src migration/src \
    && echo "fn main() {}" > src/main.rs \
    && echo "fn main() {}" > migration/src/main.rs \
    && touch migration/src/lib.rs

RUN cargo build --release -p migration \
    && cargo build --release

RUN rm -rf src migration/src

COPY src ./src
COPY migration ./migration

RUN cargo build --release -p migration \
    && cargo build --release

FROM debian:trixie

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/migration /app/migrate
COPY --from=builder /app/target/release/windwatcher /app/windwatcher

EXPOSE 8080

CMD ["sh", "-c", "\
    ./migrate && \
    exec ./windwatcher \
    "]
