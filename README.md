<p align="center"><img src="assets/icon.svg" alt="The Flode icon, an F made of wired nodes" width="128"></p>

# Flode

A synth you build by wiring nodes together. Add oscillators, envelopes, filters, LFOs and effects
to a canvas, drag wires between them, and play it from your DAW or the built-in keyboard. It
builds as a VST3, a CLAP plugin, and a standalone app. It is written in Rust with
[nih-plug](https://github.com/robbert-vdh/nih-plug) and draws its own interface with iced.

![Dusk Pad, a patch with macro knobs driving a filter and a reverb](docs/images/dusk-pad.png)

## Where the idea comes from

Flode borrows its way of working from the node editors in [Blender](https://www.blender.org): the
shader editor, geometry nodes, and the compositor. In those, you describe a result by wiring small
pieces together instead of filling in one big panel, and you can see how everything connects. A
synth suits that well, since a patch is already a signal path.

A few habits carried over directly:

- Sockets are colored by what they carry, and a wire only joins sockets of the same kind. Grey is
  audio, green is control, and purple is MIDI notes.
- A node can fold down to its sockets, packed close together, so a big patch stays on one screen.
- Dropping a node onto a wire splices it into that wire.
- `M` mutes a node, `Shift+D` duplicates it and lets you place the copy, and the right-click menu
  has the same actions.
- The Map node plays the role of Blender's Map Range and color ramp, rescaling one signal into
  another's range.

Flode is not affiliated with Blender or the Blender Foundation.

## What is in it

**Nodes**

- **Oscillator.** Sine, saw, square, triangle and noise (white, pink, brown, Geiger). Up to seven
  unison voices with detune and stereo spread, pitch controls, and warp modes (bend, sync, PWM,
  asymmetry, flip, mirror, quantize). Each has its own level and pan.
- **Envelope, LFO and Map.** An envelope shapes an oscillator's amplitude or drives any knob. The
  LFO runs free or locks to the host's tempo. A map rescales a control signal through a curve you
  can drag. The curve's four values and an Offset, which pushes the whole curve up or down, are
  also knobs in a folded Range section, and a control wire can drive any of them, so the handles
  follow it. Maps chain to any depth, including into another map's values. The Bipolar checkbox
  gives the output as a center and a range in place of Out min and Out max. Center is the middle
  of the swing, and Range is how wide it is, so a 30 percent range centered at 50 gives 35 to 65.
  A negative range runs the swing backward and inverts the map. Both knobs take control wires. The
  curve's handles still drag the two ends, and they set Center and Range. A bipolar map has no
  Offset, since Center moves the swing the same way. Switching converts the values, with any
  offset folded into Center, so the signal does not change, but the wires on the knobs that go
  away are dropped.
- **Math.** Does arithmetic on two control signals and makes one, like Blender's Math node. A
  dropdown picks the operation: add, subtract, multiply, divide, power, minimum, maximum,
  difference, average, greater than, less than, or modulo. A and B are knobs with pins. Wire a
  driver into a pin to use it as that input, or leave it unwired to use the knob's number as a
  constant. Signals run from 0 to 1 and the result stays in that range. Divide and modulo by zero
  give 0. Power raises A to a power that B sets, a quarter with B at 0, 1 at 50 percent, and 4 at
  100 percent. The output can drive any knob, including a map's values or another math node.
- **Filter.** Low pass, high pass and band pass, run once per voice, so a filter envelope follows
  each note on its own.
- **Mix.** Crossfade, AM, ring modulation, FM, or Add between two signals.
- **Effects.** Distortion (soft, hard, tube, fold, crush), compressor with a multiband mode,
  reverb, delay with tempo sync and ping pong, chorus, phaser, and flanger.
- **EQ.** Four bands on a graph. Drag a point to set that band's frequency (left and right) and
  gain (up and down). Each band also has a folded section with Freq, Gain and Q knobs, which are
  the same values as the point, so a knob moves the point and a drag turns the knob. Every knob
  has a pin, and a point follows a wire that drives it. The icon in each section header switches
  the two outer bands between a shelf and a cut, and the two inner bands between a peak and a
  notch. A cut and a notch have no gain, so their points move sideways only. Double click a point
  to reset its band. The Analyzer checkbox under the graph draws the spectrum of the EQ's output
  behind the curve.
- **MIDI Input.** Sends notes to oscillators, and holds mono, glide, and the pitch wheel, mod wheel,
  velocity and aftertouch controllers.
- **Macro.** Four knobs that are host parameters, so your DAW can automate them. Each has a pin
  that can drive any knob in the graph.
- **Output.** Up to four slots, each its own chain with its own gain, and a master gain. A level
  meter sits in the corner. The MIDI light beside it flashes when a note reaches the plugin, from
  the host or from the key strip, so a silent patch can be told from a note that never arrived.

**Wires.** Wires come in three colors. Grey is audio, green is control (envelopes, LFOs, macros and
controllers driving knobs), and purple is MIDI notes. A wire only joins ports of the same kind.
A knob with a green pin can be driven by any control wire, and it then shows the live value.

**Knobs.** Drag a knob up or down to turn it. Double click it to type an exact value in the
units the node shows, such as Hz, dB, or a percentage. Enter sets it, Escape cancels, and a click
elsewhere keeps what you typed. Option (Alt) and click puts a knob back to its default.

**Viewers.** The oscillator, envelope, LFO and filter each draw a small picture of their shape: the
waveform, the ADSR curve, and the filter's response from 20 Hz to 20 kHz. The picture follows the
knobs. It also follows a wire that drives a knob, so a filter with an LFO on its cutoff shows the
curve sweeping. While a note plays, the envelope and LFO viewers put an orange dot on the curve
where the signal is now. The envelope dot follows the newest note, and it rests in the middle of
the sustain hold. A noise LFO has no dot.

**Editing.**

- Click a node header to select it and drag it to move it. Drag a node onto a wire to splice it in.
- Shift+click adds a node to the selection or drops it from it. Dragging over empty canvas draws a
  box that selects every node it touches, and Shift keeps the current selection under it. Dragging
  one selected node's header moves the whole group. Right-click on a selected node, then Delete or
  Bypass, acts on the group. Auto pack, also in that menu, lines the group up in columns by its
  wires, with sources on the left. Only a single node splices into a wire.
- A map or math node splices into a wire that drives a knob. The old source feeds the map's shape
  pin, or input A on a math node, and the node's output drives the knob. A math node keeps its
  other input at its knob's number, so splicing one in changes the signal, unlike a map at its
  defaults.
- A node dropped on a wire needs room. If the two nodes on that wire are closer than the new node
  and a margin on each side, the node after it, and everything that node feeds, ease to the right.
- Shift and right drag from one node to another connects them. The orange line follows the cursor,
  and the node it would connect to is outlined. Only pins that carry the same kind of signal can
  join. A pair with the same name wins, such as Shape to Shape. Otherwise the first pair whose
  input is free is used, which puts audio on the first empty audio input or output slot. If the
  first node has nothing to give the second, the wire goes the other way.
- Fold the sections of a tall node, or collapse the whole node down to its pins.
- `M` mutes a source or bypasses a processor. `Shift+D` duplicates the selected node. Both are also
  in the right-click menu.
- `Shift+A` opens a search box at the cursor. Type part of a node's name, pick a result with the
  arrow keys and Enter or a click, and the node follows the cursor until you click to place it.
  Escape or a click outside closes the box.
- Scroll to pan and pinch to zoom.
- Click the strip at the bottom to play notes, in the plugin and in the standalone app. The strip
  also shows the notes the host sends. The computer keyboard plays notes in the standalone app
  only, since a host takes the keys in a plugin window. The strip covers three octaves. In the
  standalone app a blue bar over the keys marks the ones the computer keyboard plays. The octave
  buttons at the strip's left, and the Z and X keys, move the strip up or down by an octave, up to
  three.

**Presets.** The Presets button saves and loads whole patches, sorted into categories. See
[Presets](#presets) below.

![An FM bell lead, with an envelope driving the FM amount](docs/images/bell-lead.png)

## Install

Flode has been built and tested on macOS, and used in Ableton Live 12. Other systems may work but
are untested.

You need a recent stable Rust toolchain and a network connection for the first build, since
nih-plug is fetched from git.

```sh
./build.sh --release
```

The bundles are written to `target/bundled/`. The bundler has no icon option, so `build.sh` runs
`assets/apply-icon.sh` after it to add `assets/icon.icns` to each bundle. To regenerate the icon
from `assets/icon.svg`, run `sh assets/make-icns.sh`. Then copy the bundles where your host looks
for them:

```sh
cp -R target/bundled/flode.vst3 ~/Library/Audio/Plug-Ins/VST3/
cp -R target/bundled/flode.clap ~/Library/Audio/Plug-Ins/CLAP/
```

Restart or rescan in your DAW. The standalone app comes from a debug bundle
(`./build.sh`), and `./build_and_watch.sh` rebuilds and relaunches it whenever a source file
changes.

## Using it

1. Add nodes from the pill at the top, or start from a preset. A new graph has a MIDI Input, an
   Oscillator, an Envelope and an Output already wired.
2. An oscillator plays only the notes wired into its MIDI pin, so it is silent until a MIDI Input
   is connected.
3. Wire audio into an output slot to hear it.
4. Audio can go to any audio input. Effects run once, on the summed voices, and anything an effect
   feeds runs on the summed voices too. That lets two effect chains run in parallel from one
   oscillator and meet in a Mix. Add sums them, and Crossfade blends them. A filter or a mix after
   an effect is one shared instance, so a knob that an envelope drives follows the newest note. FM
   needs each note's pitch, so an FM mix after an effect passes its first input through, and its
   row in the dropdown is dimmed. You can have four of each kind of effect, and four filters after
   an effect.
5. A graph has one Output node and one Macro node. They cannot be added a second time or duplicated.

Mono, glide and "glide on every note" are on the MIDI Input node. In mono mode a new note cuts the
tail of the last one short.

![The Reese preset, a mono bass with an LFO on the filter](docs/images/reese.png)

## Presets

Presets are files in `~/Library/Application Support/Flode/Presets`, one per name. Each holds the
graph and the four macro values. A category is a folder inside that one, so `Bass/Reese.flode` is
the Reese preset in the Bass category.

The Presets button opens a list with "Save as..." and "Init", then the categories, then any presets
with none. "Init" goes back to the starting patch and resets the macros, and it does not ask about
unsaved changes. A category opens a submenu of its presets when you hover it or click it. "Save as..." opens a box
with a name and a category. The existing categories show as chips you can click, or step through
with the up and down keys, so a typo does not make a new folder. A category that does not exist
yet is tagged "new". Saving under a name that already exists replaces that preset.

![The presets menu with the Pad submenu open](docs/images/presets-menu.png)
![The save box, with the existing categories to pick from](docs/images/save-preset.png)

Six factory presets are built in code in `src/editor/factory/`: Sub Bass and Reese, Pulse Lead and
Bell Lead, Warm Pad and Dusk Pad. To write them into your preset folder:

```sh
cargo test --lib write_factory_presets -- --ignored
```

## Development

```sh
cargo test
```

The tests cover the graph model, the compiler that turns a graph into a patch, the engine, every
effect, and the editor's input handling. The audio thread must never allocate, and `nih-plug` is
built with `assert_process_allocs`, so a debug build panics on an allocation there. A test plays a
patch that runs all eight effects, the EQ's analyzer, and chained map and math nodes with
allocation forbidden.

Four environment variables help when taking screenshots. `FLODE_PRESET` (`Category/Name`) starts the
editor with a preset loaded, `FLODE_UI` (`presets`, `presets:Bass`, `save`, `select:2`, `select:0,2`) opens part
of the interface or selects nodes, `FLODE_PRESET_DIR` points the editor at another preset folder,
and `FLODE_NOTE` (a MIDI note number) holds that note down from the start, so the viewers have a
live signal. They are read once, at start. See `src/editor/demo.rs`.

### Layout

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
docs/             design notes
```

A graph compiles to a small arena of stages for each output slot, in signal order. Each voice runs
the per-voice stages of the arena. The stages on the bus, which are the effects and anything an
effect feeds, run once on the summed voices after it. `docs/effects-plan.md` has the details for
the effects.

`vendor/` holds two crates the build patches: `baseview`, for a nil-window check on newer macOS,
and `iced_baseview`, for a method that needs a newer baseview API. They keep their own licenses.

## Status

Flode is young. The DSP is covered by tests for level, stability, filter and delay behavior, and
allocation on the audio thread, but how it sounds is still being tuned by ear. Known limits:

- There are four instances of each effect kind, and four filters that run after an effect. A fifth
  passes its input through.
- A patch runs up to 24 map and math nodes together. One past that makes no signal, so a knob wired
  to it is not driven. A map with nothing wired to its input makes no signal either, and a loop of
  maps makes none. A loop of math nodes is cut where it closes, and the node that would read itself
  uses its own knob instead.
- The distortion adds about 15 samples of latency when its mix is above zero. Parallel chains are
  not delay compensated, so a distorted chain merged with a clean one can comb filter a little.
- The FM mix delays the signal by a few cycles of the note, so that knob moves do not bend the
  pitch.
- The preset list has no scrolling, delete, or rename. Delete the files to remove a preset.
