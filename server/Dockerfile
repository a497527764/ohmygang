# syntax=docker/dockerfile:1

# ============================================================
# OH-MY-GANG! 自有云端服务 —— 构建镜像
# 一个端口同时承载：Nostr 风格 WebSocket 中继 + 6 位短码 HTTP API
# ============================================================

# ---- 构建阶段 ----
FROM rust:1-slim-bookworm AS builder
WORKDIR /build

# rusqlite 使用 bundled 特性，需要在构建期把 SQLite 从 C 源码编译出来，
# 因此必须装 build-essential / pkg-config。libssl-dev 给 tokio-tungstenite 用。
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 先拷贝依赖清单，最大化利用构建缓存（仅当 Cargo 文件变动时才重新编译依赖）
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin ohmygang-relay

# ---- 运行阶段 ----
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ohmygang-relay /app/ohmygang-relay

# Zeabur 会自动注入 PORT（容器内的明文端口）；这里给个默认值便于本地跑。
# 注意：Zeabur 边缘会做 HTTPS/WSS 终止，容器内部只跑明文 HTTP/WS。
ENV PORT=8080
ENV DATA_DIR=/data/ohmygang.db

EXPOSE 8080

# 数据库持久化目录。在 Zeabur 控制台给该服务挂一个 Volume 到 /data，
# 否则容器重建后队伍/短码记录会丢失。
VOLUME ["/data"]

CMD ["/app/ohmygang-relay"]
