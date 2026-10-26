# Written by Kone Tshivhinda & Claude
# The ClearCut server (the API, the web page and the job runner) and the operator command.
# Build:  docker build -t clearcut-server .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
# The web page and the prompts are compiled into the binary (include_str!), so nothing else is needed at run time.
RUN cargo build --release -p clearcut-server -p clearcut-admin

FROM debian:bookworm-slim
# CA certificates: the server talks HTTPS to Anthropic.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin clearcut \
    && mkdir /data && chown clearcut /data
COPY --from=build /src/target/release/clearcut-server /src/target/release/clearcut-admin /usr/local/bin/
USER clearcut
WORKDIR /app
ENV CLEARCUT_STORAGE_DIR=/data CLEARCUT_BIND=0.0.0.0:8080
EXPOSE 8080
CMD ["clearcut-server"]
