#!/bin/sh
# Quita Spoti. Tu sesión de Spotify y la config de ncspot quedan; borralas a mano si querés.
set -eu
data="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -rf "$data/spoti" "$data/fonts/spoti"
rm -f "$HOME/.local/bin/spoti" "$data/applications/spoti.desktop"
echo "Spoti desinstalado. Sesión y caché: ${XDG_CACHE_HOME:-$HOME/.cache}/ncspot"
