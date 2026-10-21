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
  asymmetry, flip, mirror, quantize). Each has its own level and pan. A held note follows the
  Unison and Detune knobs, with copies fading in and out as the count changes. Phase only sets
  where a note's copies start, so it takes effect on the next note.
- **Envelope, LFO and Map.** An envelope shapes an oscillator's amplitude or drives any knob. Its
  curve has three handles you can drag. The first sets Attack, the second sets Decay sideways and
  Sustain up and down, and the third sets Release. They turn the same knobs, so a knob or a control
  wire moves its handle, and double clicking a handle resets its knobs. Each stage's width follows
  its own knob only, so the time axis is fixed and a long release shows as a long tail. The
  LFO runs free or locks to the host's tempo. Its Wave list has a Custom wave, a shape you draw as
  a list of points in the viewer. Double click empty space in the viewer to add a point there, and
  double click a point to remove it. Drag a point to move it. A curve holds up to 16 points, and
  the two end points only move up and down. The points are saved with the patch and with presets.
  The One shot checkbox makes the LFO play exactly one cycle from each note and then hold its end
  value until the next note, which restarts it. One shot restarts on every note whether or not
  Retrigger is on, and it ignores the playhead lock of Sync. With Sync on a note division and a
  tempo from the host, the cycle is that division long. Otherwise it takes 1 / Rate seconds.
  Before the first note it sits at the start of the shape. A saw ends high, and a Custom wave ends
  on its last point. A noise LFO in one shot mode draws one new random value at each note and
  holds it. A map rescales a control signal through a curve you
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
- **Gain.** A utility with a Gain knob from -60 dB (silent) to +24 dB, a Pan knob, and an Invert
  checkbox that flips the polarity of both channels. Pan is a balance, not a mono panner. At
  center both channels pass at unity, and moving it turns the far channel down to silence at the
  edge while the near channel stays at unity. Both knobs have pins. A move of a knob or a click on
  Invert eases over about 10 ms, so it does not click. Like a filter, it runs once per voice, and
  after an effect it runs once on the summed voices, where you can have four.
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
- **Monitor.** Listens to one audio wire and shows it on a screen, with no output and no knobs. A
  dropdown picks the mode. Scope draws the left and right waveform, held steady on a rising zero
  crossing. Spectrum draws the level of each band on a log axis from 20 Hz to 20 kHz. Spectrogram
  scrolls the spectrum over time, with level as color. Levels shows the RMS and peak of each
  channel on a bar, a clip light that stays lit for a second, and a strip of the last few seconds.
  Vectorscope plots left against right, with a correlation bar from -1 to +1 under it. A monitor
  shows nothing until its wire leads to an output slot.
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
the sustain hold. A noise LFO has no dot. A one shot LFO's dot stops at the end of the curve.

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
- Drag from an input pin that has no wire and drop on an output pin to wire the two from the
  input end. Drop on empty canvas instead and the search box from `Shift+A` opens, with only the
  nodes that can feed that input. The one you pick is added and wired to it, and follows the
  cursor until you click to place it. An input that already has a wire still detaches it when you
  drag from it.
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
   oscillator and meet in a Mix. Add sums them, and Crossfade blends them. A filter, a gain, or a
   mix after an effect is one shared instance, so a knob that an envelope drives follows the newest
   note. FM
   needs each note's pitch, so an FM mix after an effect passes its first input through, and its
   row in the dropdown is dimmed. You can have four of each kind of effect, and four filters and
   four gains after an effect.
5. A graph has one Output node and one Macro node. They cannot be added a second time or duplicated.

Mono, glide and "glide on every note" are on the MIDI Input node. In mono mode a new note cuts the
tail of the last one short.

![The Reese preset, a mono bass with an LFO on the filter](docs/images/reese.png)

## Groups

A group is a node that holds a graph of its own. Select some nodes and press `Shift+G` to turn
them into one. Each wire that crossed the edge of the selection goes through a pin on the group.
`Shift+Alt+G` breaks the selected group back into its nodes and joins the wires that ran through
its pins. Ableton Live takes `Cmd+G` and `Cmd+Shift+G` for its track groups, which is why these
are not the keys. They also work in the standalone app and in hosts that pass them on, as `Cmd+G`
and `Cmd+Shift+G` (`Ctrl` on Windows and Linux). Both are also in the right-click menu, where a group node
offers Open, Rename, Bypass, Ungroup, Duplicate, Save group and Delete, and a selection of several nodes offers Group.

- `Tab` opens the selected group. With no group selected, it closes the open one. `Esc` closes the
  open group too, or closes an open menu first. Double clicking a group's header, or clicking the
  box icon in it, opens it as well. A text box such as Rename or the search box keeps `Tab` and
  `Esc` for itself.
- An open group floats in a pane over a dimmed backdrop, with its path in the title strip, such as
  `Main > E Piano > Reverb Tail`. Scroll and pinch work inside the pane. The close button, or a
  click on the dimmed margin, closes it. A drag that starts on the margin does not. Groups open
  inside groups, one pane at a time. The sound keeps playing while a group is open, and the pill
  and the key strip stay in use.
- Inside, the **Group Input** node holds the pins that take a signal in, and the **Group Output**
  node holds the pins that send one out. Each has an empty pin below its last one. Drop a wire from
  an output on the empty pin of Group Output, or drag from an unwired input back to the empty pin
  of Group Input, and a new pin appears with that wire on it. Dragging out of an empty pin onto an
  input, or back from the empty pin of Group Output onto an output, works too. A pin on Group Input
  can feed any number of inputs. A pin on Group Output takes one wire.
- A pin carries one kind of signal, which it takes from the wire that made it. Audio pins are grey,
  control pins are green, and notes pins are purple. Right-click a pin, on the group node or inside,
  for Rename and Remove pin. Rename opens a box at the pin. `Enter` keeps the new name, `Esc`
  cancels, and an empty name keeps the old one. Removing a pin drops its wires on both sides. A
  new pin takes its name from the first wire that reaches it. A pin that feeds a filter's Cutoff
  becomes "Cutoff", and a pin that takes an LFO's output becomes "Shape". Only a pin that still has
  its starting name, such as "In 2", is renamed, so a name you typed stays, and a second wire does
  not rename it. A name that is taken gets a number.
- Press `M` on a selected group, or pick Bypass in its menu, to switch it off. A switched-off group
  hands each input to the output of the same kind in the same order, so audio goes straight
  through, and an output with no input to take goes silent. Its nodes make no signal and use no
  LFOs, envelopes or effect slots. The header says "bypass", or "mute" when nothing passes.
- Right-click a group node and pick Rename to name it, or click its name in the pane's title.
- A control pin that feeds a knob inside the group shows that knob on the group node, with the
  pin's name. It reads and snaps like the knob it feeds, so a pin on a Cutoff reads in Hz. Drag
  it, double click to type a value, or Alt-click to reset it. The value is written into every
  knob inside that the pin feeds, and those knobs are locked while the pin feeds them. When a wire
  runs into the pin, the knob on the group shows as driven and locks, and the knobs inside keep
  the last value. A new group starts each knob at the value of the knob it feeds, so grouping
  never changes the sound. Inside the open group, the Group Input node shows the same knob on the
  same pin. It edits the same value, and it locks when the pin is wired outside. A pin that feeds
  no knob, such as one that feeds a shape pin or another group's pin, shows none. A group node
  shows up to 8 of these knobs. A pin of a group inside a group shows no knob when it is fed by
  a pin of the group around it, and the value is not passed through the levels.
- Groups save with the patch and with presets, and a group copies with everything in it.

### Saving a group to a file

A group can go in a file of its own, so other patches can use it.

- **Save.** Right-click a group node and pick Save group, or click the disk icon in the title
  strip of an open group. The system save dialog opens in the Groups folder with the group's name
  as the file name. Nested groups save the same way. The file holds the group as it is when you
  pick Save group, with its nodes, wires, pins, face knob values and any groups inside it. The
  open or closed state does not matter. If you type a different file name, that name is the
  group's name in the file. The group in your patch keeps its own name.
- **Where.** Files are `<Name>.flodegroup` in `~/Library/Application Support/Flode/Groups`, a
  folder next to `Presets`, so a group file is never taken for a preset category. On Linux it is
  `~/.local/share/flode/groups`. A folder inside Groups works as a category does for presets, and
  the lists show the group as `Category / Name`. The folder is made on the first save. To delete
  or rename a saved group, delete or rename its file in the folder.
- **Load.** Press `Shift+A` and type a name. Saved groups are listed with the node kinds, ranked
  the same way, and marked with a dim "Group" on the right. When a group and a node kind match
  equally well, the node kind comes first. An empty search shows node kinds only, so type part of
  the name. The Groups button on the pill lists every saved group. Either way the group follows
  the cursor until you click, like any added node. While a group is open, the copy goes inside
  the open group.
- **Feeding an input.** Drag a wire back from an input and drop it on empty canvas. The box then
  lists the saved groups that have an output pin with the input's kind of signal, and the one you
  pick is wired from its first such pin.
- **A load is a copy.** The patch holds the whole group, so editing or deleting the file later
  changes nothing in patches that already used it. The lists are read from the folder each time
  they open, so a group saved a moment ago shows at once.
- **Limits.** An Output or Macro node cannot be in a group, so a file that holds one is refused.
  So is a file that is not a group, is from a newer Flode, or has a wire to a pin that is not
  there. The status line under the pill says why. The Groups list shows as many rows as fit above
  the key strip, then notes how many more are not shown. It does not scroll, the same as the
  preset list, so keep the folder short or use the search.

The Macro node can name its knobs. Right-click a row and pick Rename, and an empty name puts the
default back. The name shows on the node only. The host still lists the parameters as Macro 1 to 4.

## Presets

Presets are files in `~/Library/Application Support/Flode/Presets`, one per name. Each holds the
graph and the four macro values. A category is a folder inside that one, so `Bass/Reese.flode` is
the Reese preset in the Bass category.

The Presets button opens a list with "Save as..." and "Init", then the categories, then any presets
with none. "Init" goes back to the starting patch and resets the macros, and it does not ask about
unsaved changes. A category opens a submenu of its presets when you hover it or click it.

"Save as..." opens the system save dialog. It starts in the preset folder, or in the current
preset's category folder, and suggests the current name. The file name is the preset's name, and
Flode adds `.flode` to it if you leave that off. A folder directly inside the preset folder is the category,
so saving into `Bass` files the preset under Bass in the list, and a new folder makes a new
category. A file saved directly in the preset folder has no category. A file saved anywhere else
is written where you chose, and it works, but it does not appear in the list. Saving over an
existing file replaces that preset. Canceling the dialog does nothing.

![The presets menu with the Pad submenu open](docs/images/presets-menu.png)

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
patch that runs all eight effects, the EQ's analyzer, monitors in every mode, and chained map and
math nodes with allocation forbidden.

Four environment variables help when taking screenshots. `FLODE_PRESET` (`Category/Name`) starts the
editor with a preset loaded, `FLODE_UI` (`presets`, `presets:Bass`, `select:2`, `select:0,2`) opens part
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

- There are four instances of each effect kind, four filters that run after an effect, and four
  Gain nodes that run after an effect. A fifth passes its input through. The same goes for
  monitors, which show nothing past the fourth.
- A patch runs up to 8 LFOs and 8 envelopes, counted in the order the nodes were added. One past
  that is dimmed and tagged Over limit, and makes no signal.
- A monitor takes one stage of its slot's 16. If the monitors push a slot past the limit, that slot
  runs without them rather than going silent.
- A patch runs up to 24 map and math nodes together. One past that makes no signal, so a knob wired
  to it is not driven. A map with nothing wired to its input makes no signal either, and a loop of
  maps makes none. A loop of math nodes is cut where it closes, and the node that would read itself
  uses its own knob instead.
- The distortion adds about 15 samples of latency when its mix is above zero. Parallel chains are
  not delay compensated, so a distorted chain merged with a clean one can comb filter a little.
- The FM mix delays the signal by a few cycles of the note, so that knob moves do not bend the
  pitch.
- The limits above count what a group holds. That covers LFOs, envelopes, the effect pools, and the
  stages in a slot. Every use of a group counts, so a group copied three times counts its nodes
  three times.
- The Output and Macro nodes cannot go inside a group, and neither can a group's own edge nodes. A
  group node cannot be added from the add menu or the search box. Make one with `Shift+G`.
- The preset list has no scrolling, delete, or rename. Delete the files to remove a preset.
