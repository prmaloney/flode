#!/bin/sh
# Build the macOS release downloads into dist/: an installer (.pkg) and a zip of the plain bundles.
# Both hold a universal (Apple silicon + Intel) build of the VST3, CLAP, Audio Unit, and standalone
# app. CI runs this; it works the same locally, once `rustup target add x86_64-apple-darwin
# aarch64-apple-darwin` has been run.
#
# Usage: assets/package-macos.sh <label>
# The label goes into the file names, e.g. `0.1.0` makes Flode-0.1.0-macOS.pkg.
#
# Nothing here is signed with a Developer ID or notarized, so Gatekeeper still warns about the
# installer. Files an installer writes carry no quarantine flag, so the plugins it installs load
# without the `xattr` step that the plain zip needs.
set -e
cd "$(dirname "$0")/.."

label=${1:?usage: package-macos.sh <label>}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=target/bundled
dist=dist
work=target/package-macos

# nih-plug's bundler builds both architectures and lipos them together.
cargo run --package xtask --release -- bundle-universal flode --release
sh assets/apply-icon.sh
# There is no universal libflode.dylib in target/, but the VST3 binary is exactly that library.
FLODE_LIB="$out/flode.vst3/Contents/MacOS/flode" sh assets/bundle-au.sh --release

rm -rf "$work" "$dist"
mkdir -p "$work" "$dist"

# The zip holds the bundles as they are, in a folder named for the release.
zipdir="$work/Flode-$label-macOS"
mkdir -p "$zipdir"
for b in flode.vst3 flode.clap flode.component flode.app; do
  cp -R "$out/$b" "$zipdir/"
done
ditto -c -k --norsrc --keepParent "$zipdir" "$dist/Flode-$label-macOS.zip"

# One component package per format, so the installer can offer them as separate choices.
# pkgbuild marks bundles as relocatable by default, which lets the installer "upgrade" a copy it
# finds anywhere on disk, including in target/. The component plist turns that off.
component() { # <bundle> <install location> <id suffix>
  bundle=$1 location=$2 id=$3
  root="$work/root-$id"
  mkdir -p "$root"
  cp -R "$out/$bundle" "$root/"
  cat > "$work/$id.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict>
    <key>RootRelativeBundlePath</key><string>$bundle</string>
    <key>BundleIsRelocatable</key><false/>
    <key>BundleIsVersionChecked</key><false/>
    <key>BundleHasStrictIdentifier</key><false/>
    <key>BundleOverwriteAction</key><string>upgrade</string>
  </dict>
</array>
</plist>
EOF
  pkgbuild --root "$root" --component-plist "$work/$id.plist" \
    --identifier "com.prmaloney.flode.$id" --version "$version" \
    --install-location "$location" "$work/flode-$id.pkg"
}
component flode.vst3 /Library/Audio/Plug-Ins/VST3 vst3
component flode.clap /Library/Audio/Plug-Ins/CLAP clap
component flode.component /Library/Audio/Plug-Ins/Components au
component flode.app /Applications app

cat > "$work/distribution.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>Flode $version</title>
  <options customize="allow" require-scripts="false" hostArchitectures="arm64,x86_64"/>
  <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
  <choices-outline>
    <line choice="vst3"/>
    <line choice="clap"/>
    <line choice="au"/>
    <line choice="app"/>
  </choices-outline>
  <choice id="vst3" title="VST3" description="The VST3 plugin, for Ableton Live, Reaper, Bitwig and most other hosts.">
    <pkg-ref id="com.prmaloney.flode.vst3"/>
  </choice>
  <choice id="clap" title="CLAP" description="The CLAP plugin, for hosts that support CLAP.">
    <pkg-ref id="com.prmaloney.flode.clap"/>
  </choice>
  <choice id="au" title="Audio Unit" description="The Audio Unit, for Logic Pro and GarageBand.">
    <pkg-ref id="com.prmaloney.flode.au"/>
  </choice>
  <choice id="app" title="Standalone app" description="Flode as an app of its own, installed in Applications.">
    <pkg-ref id="com.prmaloney.flode.app"/>
  </choice>
  <pkg-ref id="com.prmaloney.flode.vst3" version="$version" onConclusion="none">flode-vst3.pkg</pkg-ref>
  <pkg-ref id="com.prmaloney.flode.clap" version="$version" onConclusion="none">flode-clap.pkg</pkg-ref>
  <pkg-ref id="com.prmaloney.flode.au" version="$version" onConclusion="none">flode-au.pkg</pkg-ref>
  <pkg-ref id="com.prmaloney.flode.app" version="$version" onConclusion="none">flode-app.pkg</pkg-ref>
</installer-gui-script>
EOF
productbuild --distribution "$work/distribution.xml" --package-path "$work" \
  "$dist/Flode-$label-macOS.pkg"

ls -l "$dist"
