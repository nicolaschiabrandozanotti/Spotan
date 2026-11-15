#!/bin/sh
# Instala Spoti solo para tu usuario (no pide sudo).
set -eu
src=$(cd "$(dirname "$0")" && pwd)
data="${XDG_DATA_HOME:-$HOME/.local/share}"
conf="${XDG_CONFIG_HOME:-$HOME/.config}/ncspot"
mkdir -p "$data/spoti" "$HOME/.local/bin" "$data/fonts/spoti" "$data/applications" "$conf"
install -m 755 "$src/ncspot" "$data/spoti/ncspot"
install -m 755 "$src/spoti" "$HOME/.local/bin/spoti"
cp "$src/fonts/"* "$data/fonts/spoti/"
command -v fc-cache >/dev/null 2>&1 && fc-cache -f "$data/fonts/spoti" >/dev/null 2>&1 || true
sed "s|^Exec=.*|Exec=$HOME/.local/bin/spoti|" "$src/spoti.desktop" > "$data/applications/spoti.desktop"
if [ -e "$conf/config.toml" ]; then
    cp "$src/config.toml" "$conf/config.spoti.toml"
    echo "Ya tenías $conf/config.toml: no lo toqué. El tema de Spoti quedó en config.spoti.toml."
else
    cp "$src/config.toml" "$conf/config.toml"
fi
missing=$(ldd "$data/spoti/ncspot" 2>/dev/null | grep 'not found' || true)
if [ -n "$missing" ]; then
    echo "Faltan bibliotecas del sistema:"; echo "$missing"
    echo "Debian/Ubuntu: sudo apt install libpulse0 libcairo2 libxkbcommon0"
    echo "Fedora:        sudo dnf install pulseaudio-libs cairo libxkbcommon"
    echo "Arch:          sudo pacman -S libpulse cairo libxkbcommon"
fi
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Agregá ~/.local/bin a tu PATH para usar el comando spoti.";; esac
echo "Listo. Abrilo con: spoti (o buscá Spoti en tu menú de aplicaciones)."
