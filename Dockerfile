FROM rust:slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY static/ static/

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/minesweeper-agent /app/minesweeper-agent
COPY --from=builder /app/static /app/static

ENV OCR_URL=http://ocr:5001

EXPOSE 8080

CMD ["./minesweeper-agent", "serve", "--port", "8080", "--ocr-url", "http://ocr:5001"]
