#!/bin/sh
# Make target/bundled/flode.component, the Audio Unit for Logic and GarageBand. The plugin library
# already carries the Audio Unit entry point, wrapped around the CLAP one by clap-wrapper, so the
# bundle is the library plus an Info.plist that tells macOS what it is. nih-plug's bundler has no
# Audio Unit support, so build.sh runs this after it. Pass the same arguments as the bundler, since
# `--release` picks which build's library is used. The bundle is signed ad hoc at the end.
set -e
cd "$(dirname "$0")/.."

profile=debug
for arg in "$@"; do
  [ "$arg" = "--release" ] && profile=release
done

lib="target/$profile/libflode.dylib"
[ -f "$lib" ] || { echo "No $lib to make an Audio Unit from"; exit 1; }

bundle=target/bundled/flode.component
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$lib" "$bundle/Contents/MacOS/flode"
cp assets/Info-au.plist "$bundle/Contents/Info.plist"
cp assets/icon.icns "$bundle/Contents/Resources/icon.icns"
codesign -f -s - "$bundle" 2>/dev/null || echo "Could not sign $bundle"
echo "Created an Audio Unit bundle at '$PWD/$bundle'"
