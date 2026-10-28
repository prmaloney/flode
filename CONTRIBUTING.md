# Contributing to Flode

Thanks for taking an interest in Flode. This is a small project, so the process is light.

## Getting set up

Flode is a Rust project built with [nih-plug](https://github.com/robbert-vdh/nih-plug) and draws
its interface with iced. You'll need a recent stable Rust toolchain (install via
[rustup](https://rustup.rs)).

- `./build.sh` builds the CLAP, VST3, standalone, and Audio Unit bundles into `target/bundled/`
  and applies the icon. Pass `--release` for a release build.
- `cargo run --package xtask -- bundle flode` is the underlying bundler command, if you need more
  control than `build.sh` gives you.
- `cargo test` runs the test suite.
- `cargo check` is the fastest way to see if your changes compile.

## Before opening a pull request

Two rules are non-negotiable:

- **`cargo fmt && cargo test --lib`** must pass before anything ships.
- **The audio thread must never allocate.** `nih-plug` is built with `assert_process_allocs`, so a
  debug build panics on an allocation there, and `src/engine/no_alloc.rs` enforces it in tests. Any
  new DSP work needs to respect this — no `Vec`, `Box`, `String`, or similar on the audio path.

Beyond that:

- Run `cargo clippy` and address warnings in code you touched.
- Keep the change focused. If you spot an unrelated issue while working, it's fine to mention it
  in the PR description, but prefer a separate PR for the fix.
- Add tests for new behavior where it makes sense, following the existing `*_tests.rs` pattern
  used alongside modules in `src/editor/` and `src/engine/`.

## Reporting bugs

Open an issue with:

- What you expected to happen and what happened instead.
- Steps to reproduce, including the host (standalone, or the DAW and plugin format if relevant).
- Your OS and, if it's a build problem, your Rust toolchain version (`rustc --version`).

## Suggesting features or changes to how nodes work

Flode's design leans on the idea that a patch is a visible, wired-together signal path (see the
README for where that comes from). If you're proposing a new node or a change to how wiring,
sockets, or the canvas behave, it helps to open an issue describing the idea before sending a
large PR, so we can talk through how it fits before you spend time implementing it.

## License

By contributing, you agree that your contributions will be licensed under the project's
[GPL-3.0-or-later license](LICENSE).
