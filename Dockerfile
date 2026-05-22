FROM rust:1-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY crates/ipmaze-controller/Cargo.toml crates/ipmaze-controller/Cargo.toml
COPY crates/ipmaze-controller/src crates/ipmaze-controller/src

RUN cargo build --locked --release -p ipmaze-controller

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /workspace/target/release/ipmaze-controller /usr/local/bin/ipmaze-controller

ENTRYPOINT ["/usr/local/bin/ipmaze-controller"]
CMD ["run"]
