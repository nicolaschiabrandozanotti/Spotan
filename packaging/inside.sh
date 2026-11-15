#!/bin/sh
set -eu
cargo build --locked --profile distribution --features native --bin ncspot
cargo build --locked --profile distribution --target x86_64-pc-windows-gnu --bin ncspot \
  --no-default-features --features panels,rodio_backend,crossterm_backend

out=/src/dist; rm -rf "$out/spoti-linux-x86_64" "$out/spoti-windows-x86_64"; mkdir -p "$out"
pkg=/src/packaging

l="$out/spoti-linux-x86_64"; mkdir -p "$l/fonts"
cp /target/distribution/ncspot "$l/"
cp "$pkg/linux/install.sh" "$pkg/linux/uninstall.sh" "$pkg/linux/spoti" "$pkg/linux/spoti.desktop" "$pkg/linux/LEEME.txt" "$l/"
cp "$pkg/config.toml" "$l/config.toml"
cp "$pkg/fonts/"* "$l/fonts/"
cp /src/ncspot/LICENSE "$l/LICENSE-ncspot"
chmod 755 "$l/ncspot" "$l/install.sh" "$l/uninstall.sh" "$l/spoti"

w="$out/spoti-windows-x86_64"; mkdir -p "$w"
cp /target/x86_64-pc-windows-gnu/distribution/ncspot.exe "$w/spoti.exe"
cp "$pkg/windows/"* "$w/"
# Windows Terminal's default font has no Nerd Font icons.
sed 's/^use_nerdfont = true/use_nerdfont = false/' "$pkg/config.toml" > "$w/config.toml"
cp /src/ncspot/LICENSE "$w/LICENSE-ncspot.txt"
for f in "$w"/*.txt "$w"/*.bat "$w/config.toml"; do sed -i 's/$/\r/' "$f"; done

cd "$out"
tar -czf spoti-linux-x86_64.tar.gz spoti-linux-x86_64
rm -f spoti-windows-x86_64.zip; zip -qr spoti-windows-x86_64.zip spoti-windows-x86_64
chown -R "$HOST_UID:$HOST_GID" "$out"
ls -la "$out"
