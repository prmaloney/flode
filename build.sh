set -e
cd "$(dirname "$0")"

# Builds the CLAP, VST3, and standalone bundles into target/bundled/ and adds the icon to each.
# Extra arguments go to the bundler, so `./build.sh --release` makes a release build.
cargo run --package xtask -- bundle flode "$@"
sh assets/apply-icon.sh
