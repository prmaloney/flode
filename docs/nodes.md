# Node reference

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

## Nodes

- **Oscillator.** Sine, saw, square, triangle and noise (white, pink, brown, Geiger). Up to seven
  unison voices with detune and stereo spread, pitch controls, and warp modes (bend, sync, PWM,
  asymmetry, flip, mirror, quantize). Each has its own level and pan. A held note follows the
  Unison and Detune knobs, with copies fading in and out as the count changes. Phase only sets
  where a note's copies start, so it takes effect on the next note.
- **Sampler.** Plays back a loaded audio file, one voice per held note, pitch-shifted from the note
  relative to a Root key. The Load sample row opens a file picker; it shows the loaded file's name
  once one is chosen. WAV, AIFF, FLAC, MP3, Ogg Vorbis, and AAC/M4A load, mono or stereo. Drag an
  audio file from Finder, or a clip from a DAW's timeline, onto the editor and a Sampler appears
  under the cursor with the file already loaded. Pitch tracking, on by default, shifts playback
  speed with the note; off, every note plays at the file's own speed, which suits a one-shot drum
  hit. Start sets where playback begins on every new note. Loop picks Off, Forward, or Ping-pong,
  with Loop start and Loop end marking the region. Reverse plays the file backward. Gated, the
  default, stops on note off through the same envelope every other source uses; One shot ignores
  note off and plays through to the end or loops forever. The viewer draws the loaded waveform with
  the Start, Loop start, and Loop end positions marked on it, live as the knobs move, plus a
  playhead line that sweeps across it while a voice is sounding. Like an oscillator, a Sampler is
  silent until a MIDI input or a note-routing node is wired to its Notes pin.
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
- **Filter.** Low pass, high pass, band pass, notch and peak, run once per voice, so a filter
  envelope follows each note on its own. A notch cuts the band around the cutoff and passes the
  rest. A peak is flat with Resonance at zero and lifts the band around the cutoff as Resonance
  rises, like a single EQ bell. The low and high pass have a Slope knob that steps through 6, 12,
  18 and 24 dB per octave, and the response picture follows it. It shows only for those two
  types, and it has no pin. 12 dB is the default. Those two types are a Moog-style ladder of four one-pole
  stages, with the Slope taking the output after the first, second, third or fourth stage. The
  resonance feedback runs around all four, so Resonance works at every slope. Like a ladder, it
  thins the low end a little as it rises. The band pass, notch and peak are a separate
  state-variable filter and have no slope.
- **Mix.** Crossfade, AM, ring modulation, FM, or Add between two signals.
- **Gain.** A utility with a Gain knob from -60 dB (silent) to +24 dB, a Pan knob, and an Invert
  checkbox that flips the polarity of both channels. Pan is a balance, not a mono panner. At
  center both channels pass at unity, and moving it turns the far channel down to silence at the
  edge while the near channel stays at unity. Both knobs have pins. A move of a knob or a click on
  Invert eases over about 10 ms, so it does not click. Like a filter, it runs once per voice, and
  after an effect it runs once on the summed voices, where you can have four.
- **Effects.** Distortion (soft, hard, tube, fold, crush, overdrive), compressor with a multiband mode,
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
- **Note-routing nodes.** Arpeggiator and Chord both sit between a MIDI input and an oscillator on
  the Notes wire, reshaping notes before they reach it. An oscillator reads either kind on its
  Notes pin, so one patch can play a plain chord in one slot and a reshaped one in another from the
  same keys, and they can feed each other, since both are note sources. Dropping either on a Notes
  wire splices it in, the same as an audio or control node dropped on their own kind of wire.
  Neither makes any sound of its own.
  - **Arpeggiator** reads the chord currently held and plays a pattern from it instead of the raw
    chord. Pattern picks Up, Down, Up-down, As played, or Random. Rate is free running in Hz, or
    locked to the host tempo through Sync, the same pair of knobs as an LFO's. Octaves repeats the
    pattern from one to four octaves up before it cycles back. Gate sets how much of each step's
    length the note sounds before its note off, from fully legato at 100 percent down to short and
    staccato. The pattern keeps moving forward through a chord that changes size rather than
    restarting at its first note on every key you add, so a strummed chord does not stutter.
  - **Chord** turns every note it receives into a stack of notes: the root plus the chosen chord's
    intervals, all starting together and all releasing together when the root's own note off
    arrives. Chord picks Major, Minor, Major 7, Minor 7, Dominant 7, Sus2, Sus4, Power, or Octave.
    Spread, from 0 to 100 percent, says how far apart the notes land: at 0 every note sits in the
    chord's own octave, unmoved, and at 100 each note past the root climbs one further octave above
    the last, so a three-note chord's top note ends up two octaves higher than at 0. A chord type or
    spread change while a note is still held never disturbs that note's chord, only notes played
    after the change, since each note remembers exactly what it turned on. Unlike the arpeggiator,
    Chord has no clock of its own: it reacts to notes as they arrive and let go.
- **Macro.** Four knobs that are host parameters, so your DAW can automate them. Each has a pin
  that can drive any knob in the graph.
- **Output.** Up to four slots, each its own chain with its own gain, and a master gain. A level
  meter sits in the corner. The MIDI light beside it flashes when a note reaches the plugin, from
  the host or from the key strip, so a silent patch can be told from a note that never arrived.

## Wires, knobs and viewers

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

## Editing

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
  also shows the notes the host sends. The strip covers three octaves. The octave buttons at the
  strip's left, and the Z and X keys in Keyboard mode, move the strip up or down by an octave, up
  to three.

## Editor modes

The editor is in one of two modes, shown by a small badge above the strip at the
bottom right. `Tab` switches between them, everywhere, including inside an open group.

- **Keyboard** (the default) is what the computer keyboard has always done: A, W, S, E, D, F, T,
  G, Y, H, U, J, K, O, L, P, and semicolon play notes, and Z and X shift the octave. This now
  works in the plugin as well as the standalone app; which kind of process this is no longer
  matters, only the mode does.
- **Edit** turns the piano off and hands those same keys to the editor instead:
  - `G` grabs the selection: it follows the cursor with no mouse button held. A left click or
    `Enter` sets it down where it is, splicing a lone node into the wire under it the same way an
    ordinary drag does. `Esc` or a right click cancels and snaps every moved node back.
  - `X` or `Delete` removes the selection.
  - `Command+X` (`Ctrl+X` on Windows and Linux) removes the selection and reconnects around each
    node, Blender's node editor trick of Delete and Reconnect. A node with a single clear
    through-path, such as a filter, an effect, a map, a math node, or an arpeggiator, hands what
    fed it straight to what it fed. A node with no such path, such as the output, is simply
    removed. Deleting a run of several nodes in place bypasses the whole run. It is also in the
    right-click menu, next to Delete.
  - `A` selects every node. `Alt+A` clears the selection.

  `Shift+A`, `Shift+D`, `Shift+G`, and `Shift+Alt+G` are unchanged in both modes, since holding
  Shift already kept them off the piano's keys.

**Presets.** The Presets row of the burger menu saves and loads whole patches, sorted into
categories. See [Presets](presets.md).
