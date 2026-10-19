
set -e
cd "$(dirname "$0")"

# The standalone runs from its .app bundle. `open` launches a bundle through LaunchServices,
# which brings the window to the front. A bare binary opens behind the terminal.
APP=target/bundled/flode.app
APP_PATTERN="flode.app/Contents/MacOS/flode"

stop_app() {
  pkill -f "$APP_PATTERN" 2>/dev/null || true
  # Give the old process a moment to exit before the bundle is replaced.
  sleep 0.5
}
trap stop_app EXIT INT TERM

# A fingerprint of every source file's modification time. It changes when any file is saved.
source_stamp() {
  find src Cargo.toml -type f -exec stat -f '%m %N' {} + | md5 -q
}

# The standalone defaults to 48 kHz, but the default output device here runs at 44.1 kHz.
# At a mismatch, CoreAudio returns about 9% more frames per callback than the 512 the standalone
# expects, and the backend panics on the audio thread. Matching the device rate also fixes pitch.
STANDALONE_ARGS="--sample-rate 44100"

# Rebuild the bundle and relaunch it. A failed build leaves the app closed until the next save.
rebuild_and_launch() {
  stop_app
  echo "Building..."
  if cargo run --package xtask -- bundle flode && sh assets/apply-icon.sh; then
    open -n "$APP" --args $STANDALONE_ARGS
  else
    echo "Build failed. Fix the error and save to rebuild."
  fi
}

last_stamp=""
while true; do
  stamp=$(source_stamp)
  if [ "$stamp" != "$last_stamp" ]; then
    last_stamp=$stamp
    rebuild_and_launch
  fi
  sleep 1
done
