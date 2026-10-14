#!/bin/sh
# Installs the Monolith desktop client for the current user, without root:
# the binary in ~/.local/bin, the icon and the desktop entry where desktop
# environments look for them. Run it from the unpacked folder. To remove
# the client, delete the three files it names. Tor is not installed: Monolith
# uses the Tor the system runs (README.txt).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
mkdir -p "$bin" "$apps" "$icons"

install -m 0755 "$here/monolith-desktop" "$bin/monolith-desktop"
install -m 0644 "$here/monolith.png" "$icons/monolith.png"
sed "s|^Exec=.*|Exec=$bin/monolith-desktop|" "$here/monolith.desktop" > "$apps/monolith.desktop"
chmod 0644 "$apps/monolith.desktop"

echo "Installed:"
echo "  $bin/monolith-desktop"
echo "  $icons/monolith.png"
echo "  $apps/monolith.desktop"
