#!/usr/bin/env bash
# Install (or remove) a desktop entry and icons for a Lyrixearch build run from this checkout,
# so Linux desktops show its icon and name. Installed packages (.deb/.rpm) do this themselves.
#
# Under Wayland a window cannot set its own icon: the desktop finds it through the .desktop file
# named after the window's app id. GTK sends the program name, lyrixearch-gui (checked with
# WAYLAND_DEBUG=client: xdg_toplevel.set_app_id("lyrixearch-gui")).
#
#   scripts/linux-desktop-entry.sh            install, pointing at target/debug/lyrixearch-gui
#   scripts/linux-desktop-entry.sh release    install, pointing at target/release/lyrixearch-gui
#   scripts/linux-desktop-entry.sh remove     take it out again
set -euo pipefail

APP_ID="lyrixearch-gui"
OLD_ENTRY="dev.lyrixearch.desktop"  # an earlier, wrongly named entry; removed on install
ICON_NAME="lyrixearch"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
entry="$data/applications/$APP_ID.desktop"
sizes=(16 24 32 48 64 128 256 512)

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
  command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
}

rm -f "$data/applications/$OLD_ENTRY.desktop"

if [[ "${1:-}" == "remove" ]]; then
  rm -f "$entry"
  for s in "${sizes[@]}"; do rm -f "$data/icons/hicolor/${s}x${s}/apps/$ICON_NAME.png"; done
  refresh
  echo "removed $entry and the icons"
  exit 0
fi

profile="${1:-debug}"
binary="${CARGO_TARGET_DIR:-$here/target}/$profile/lyrixearch-gui"
if [[ "${2:-}" == "--package" ]]; then
  binary=/usr/bin/lyrixearch-gui
else
  [[ -x "$binary" ]] || { echo "no build at $binary (run: cargo build -p lyrixearch-gui${profile:+ --$profile})" >&2; exit 1; }
fi

for s in "${sizes[@]}"; do
  src="$here/crates/gui/icons/${s}x${s}.png"
  [[ -f "$src" ]] || continue
  install -Dm644 "$src" "$data/icons/hicolor/${s}x${s}/apps/$ICON_NAME.png"
done
install -d "$data/applications"
cat > "$entry" <<EOF
[Desktop Entry]
Type=Application
Name=Lyrixearch
Comment=Find lyrics for your music and embed them in the files
Exec="$binary" %F
Icon=$ICON_NAME
Terminal=false
Categories=AudioVideo;Audio;
MimeType=audio/mpeg;audio/flac;audio/mp4;audio/ogg;audio/opus;audio/x-wav;audio/x-aiff;audio/x-wavpack;audio/x-ape;audio/x-ms-wma;inode/directory;
StartupWMClass=$APP_ID
EOF
if [[ "${2:-}" != "--package" ]]; then refresh; fi
echo "installed $entry (runs $binary)"
