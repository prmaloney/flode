# Development

```sh
cargo test
```

The tests cover the graph model, the compiler that turns a graph into a patch, the engine, every
effect, and the editor's input handling. The audio thread must never allocate, and `nih-plug` is
built with `assert_process_allocs`, so a debug build panics on an allocation there. A test plays a
patch that runs all eight effects, the EQ's analyzer, monitors in every mode, and chained map and
math nodes with allocation forbidden.

Four environment variables help when taking screenshots. `FLODE_PRESET` (`Category/Name`) starts
the editor with a preset loaded, `FLODE_UI` (`presets`, `presets:Bass`, `select:2`, `select:0,2`)
opens part of the interface or selects nodes, `FLODE_PRESET_DIR` points the editor at another
preset folder, and `FLODE_NOTE` (a MIDI note number) holds that note down from the start, so the
viewers have a live signal. They are read once, at start. See `src/editor/demo.rs`.

See also [CONTRIBUTING.md](../CONTRIBUTING.md) for the pre-PR checklist.

## Layout

```
src/
  lib.rs          the plugin: host events and buffers in, calls on the engine out
  dsp/            filters, oscillators, wavetables, envelopes, the spectrum analyzer, and the effects
  engine/         voices, lanes, the stage arena, LFOs, and the effect pool
  patch/          what the editor hands the audio thread: stages, control routes, the table of
                  map and math signals, and the compiler
  nodes/          the node kinds: their rows, knobs, ports, and layout
  editor/         the canvas, drawing, menus, shortcuts, saving, and the factory presets
  presets.rs      preset files and the folder they live in
xtask/            bundles the plugin formats
vendor/           patched copies of baseview and iced_baseview
docs/             this directory: design notes and the full user reference
```

A graph compiles to a small arena of stages for each output slot, in signal order. Each voice runs
the per-voice stages of the arena. The stages on the bus, which are the effects and anything an
effect feeds, run once on the summed voices after it. [effects-plan.md](effects-plan.md) has the
details for the effects.

`vendor/` holds two crates the build patches: `baseview`, for a nil-window check on newer macOS,
and `iced_baseview`, for a method that needs a newer baseview API. They keep their own licenses.

## Releases

`.github/workflows/release.yml` builds the downloads and publishes them on the GitHub Releases page.
Every push to `main` replaces the **Nightly** pre-release. To make a permanent release, bump
`version` in `Cargo.toml`, commit, then push a tag named `v<version>` (for example `v0.2.0`); the
workflow refuses a tag that does not match `Cargo.toml`. You can also run the workflow by hand from
the Actions tab to check that the builds work, without publishing.

`assets/package-macos.sh <label>` makes the macOS installer and zip into `dist/`, and runs the same
way on your machine. `assets/windows/installer.iss` is the Windows installer, built with Inno Setup
after `cargo run --package xtask --release -- bundle flode --release`. Neither is signed or
notarized.
