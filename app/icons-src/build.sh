#!/bin/bash
# Regenerates every app icon from icon.svg (the single source of truth).
#
# Outputs:
#   app/app-icon.png      - 1024px full-bleed render (the `tauri icon` input)
#   app/src-tauri/icons/* - all platform icons via `tauri icon`
#   .../icon.icns         - replaced with a macOS-proportioned variant: the
#                           artwork scaled to Apple's icon grid (824/1024)
#                           with a baked drop shadow, so the dock icon is
#                           not oversized next to native apps
#
# Needs rsvg-convert (brew install librsvg) and app/node_modules (npm install).
set -euo pipefail
cd "$(dirname "$0")"

# 1. Full-bleed 1024px render, the default `tauri icon` input.
rsvg-convert -w 1024 -h 1024 icon.svg -o ../app-icon.png

# 2. The full platform set (png, ico, icns, Square*, android/, ios/).
(cd .. && ./node_modules/.bin/tauri icon)

# 3. macOS-correct icns: pad the artwork to the Apple grid and add the
#    conventional shadow, then keep only the icns from a second run.
#    (sed anchors on the root clip group and the end of <defs> in icon.svg.)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
sed \
  -e 's|</defs>|<filter id="dockShadow" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="12"/></filter></defs>|' \
  -e 's|<g clip-path="url(#squircle)">|<rect x="100" y="112" width="824" height="824" rx="187" fill="#000000" opacity="0.3" filter="url(#dockShadow)"/><g clip-path="url(#squircle)" transform="translate(100 100) scale(0.8046875)">|' \
  icon.svg > "$tmp/icon-macos.svg"
rsvg-convert -w 1024 -h 1024 "$tmp/icon-macos.svg" -o "$tmp/icon-macos.png"
(cd .. && ./node_modules/.bin/tauri icon "$tmp/icon-macos.png" -o "$tmp/out")
cp "$tmp/out/icon.icns" ../src-tauri/icons/icon.icns

echo "done: app-icon.png and src-tauri/icons/ regenerated"
