FROM rust:1.87-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin strata-agent

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/strata-agent /usr/local/bin/strata-agent
COPY --from=builder /app/crates/strata-agent/soul.md /app/soul.md
ENV SOUL_FILE=/app/soul.md
EXPOSE 3000
CMD ["strata-agent"]
