# flode

A MIDI-driven sine wave synth, written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug). Starts as a monophonic sine oscillator with a gain control. Builds as CLAP and VST3.

## Build

```sh
cargo xtask bundle flode --release
```

Bundles are written to `target/bundled/`.

## Test

- Standalone: not set up yet. Test in a DAW for now.
- Validate the VST3 with [pluginval](https://github.com/Tracktion/pluginval).
- Reaper is a forgiving host for early testing.

## Notes

- AU is not built directly. Wrap the CLAP build with [clap-wrapper](https://github.com/free-audio/clap-wrapper) if you need it for Logic or GarageBand.
- Pick a license before publishing. The VST3 SDK and nih-plug have their own license terms.
