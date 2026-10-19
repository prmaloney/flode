#!/bin/sh
# Render assets/icon.svg into assets/icon.icns. Needs magick and iconutil (macOS).
set -e
cd "$(dirname "$0")"

SET=$(mktemp -d)/icon.iconset
mkdir -p "$SET"

# Each entry is the pixel size and the iconset file name macOS expects.
render() {
  magick -background none -density 576 icon.svg -resize "${1}x${1}" "PNG32:$SET/$2"
}
render 16 icon_16x16.png
render 32 icon_16x16@2x.png
render 32 icon_32x32.png
render 64 icon_32x32@2x.png
render 128 icon_128x128.png
render 256 icon_128x128@2x.png
render 256 icon_256x256.png
render 512 icon_256x256@2x.png
render 512 icon_512x512.png
render 1024 icon_512x512@2x.png

iconutil -c icns "$SET" -o icon.icns
rm -rf "$(dirname "$SET")"
echo "Wrote $(pwd)/icon.icns"
