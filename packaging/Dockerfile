# Bookworm's glibc 2.36 keeps the Linux binary runnable on older distros.
FROM rust:1.99.1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libpulse-dev libssl-dev libcairo2-dev libxkbcommon-dev gcc-mingw-w64-x86-64 zip \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-pc-windows-gnu
