# syntax=docker/dockerfile:1
# Build: docker build -f deploy/Dockerfile -t pvxwiki-mcp .
FROM rust:1.99.1-slim-trixie AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin pvxwiki-mcp && mkdir -p /out/data

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=builder /app/target/release/pvxwiki-mcp /usr/local/bin/pvxwiki-mcp
# Owned by the nonroot user (65532) so a fresh named volume is writable.
COPY --from=builder --chown=65532:65532 /out/data /data
ENV PVXWIKI_DATA_DIR=/data
# Every interface inside the container; compose publishes 8081 on 127.0.0.1 only.
ENV PVXWIKI_ADMIN_LISTEN=0.0.0.0:8081
EXPOSE 8080 8081
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/pvxwiki-mcp"]
CMD ["serve"]
