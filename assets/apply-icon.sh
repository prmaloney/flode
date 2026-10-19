#!/bin/sh
# Copy icon.icns into each bundle in target/bundled and point its Info.plist at it.
# nih-plug's bundler writes an empty CFBundleIconFile and has no icon option, so this runs after
# every bundle build. It is safe to run repeatedly. Editing the bundle breaks the bundler's
# ad-hoc signature, so each bundle is signed again at the end.
set -e
cd "$(dirname "$0")/.."

ICNS=assets/icon.icns
PLISTBUDDY=/usr/libexec/PlistBuddy

for bundle in target/bundled/flode.app target/bundled/flode.vst3 target/bundled/flode.clap; do
  [ -d "$bundle" ] || continue
  mkdir -p "$bundle/Contents/Resources"
  cp "$ICNS" "$bundle/Contents/Resources/icon.icns"
  plist="$bundle/Contents/Info.plist"
  "$PLISTBUDDY" -c "Set :CFBundleIconFile icon" "$plist" 2>/dev/null \
    || "$PLISTBUDDY" -c "Add :CFBundleIconFile string icon" "$plist"
  codesign -f -s - "$bundle" 2>/dev/null || echo "Could not sign $bundle"
done
