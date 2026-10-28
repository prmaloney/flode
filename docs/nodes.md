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
- Muting, duplicating and the other editing actions are a key press or a right-click away. See
  [Shortcuts and gestures](shortcuts.md).
- The Map node plays the role of Blender's Map Range and color ramp, rescaling one signal into
  another's range.

Flode is not affiliated with Blender or the Blender Foundation.

## Nodes

A node's header leans toward the color of the wire it mainly makes: green for control (LFO,
envelope, map, math, macros), purple for notes (input, arpeggiator, chord). Audio nodes keep the
neutral header, since audio is most of a graph.

- **Oscillator.** Plays a wavetable wired to its Table pin, on the viewer's row. It has no
  waveform of its own, so with nothing wired there it is silent, as it is with no notes. A new
  one is wired to a saw node. Wire a Sine, Saw, Square or Triangle node there for a plain wave,
  or any other table (see Wavetables below). Up to seven unison voices with detune and stereo
  spread, pitch controls, and warp modes (bend, sync, PWM, asymmetry, flip, mirror, quantize, PM,
  PD). Each has its own level and pan. A held note follows the Unison and Detune knobs, with
  copies fading in and out as the count changes. Phase only sets where a note's copies start, so
  it takes effect on the next note.

  The audio pin on the Warp row is read by the PM and PD warps. With another oscillator wired in,
  PM pushes the phase by that signal, with the Warp amount as the depth (up to two cycles). PD
  bends the cycle toward one end or the other as the signal swings, like the Casio CZ's phase
  distortion, with the amount as how far it can move. With nothing wired, both play the plain
  wave, and the other warps ignore the pin. The modulator is another oscillator, with its own
  Notes wire, and goes through its own envelope and level. A modulator that has passed through an
  effect is already summed across voices, so it is ignored. Shift-right-drag and the other
  automatic wiring never pick this pin. Drag a wire to it.
- **Noise.** White, pink, brown or Geiger noise, played from the notes wired into it and shaped by
  an envelope, with level and pan. It has no pitch, so no transpose, unison or warp. It lives in
  the Utility menu.
- **Wavetables.** The Wavetable menu holds the nodes that make a table for an oscillator to play.
  Table wires and pins are rose, and their pins are diamonds. A table is an ordered list of
  frames, each one cycle of a wave. Every node that makes a table has the same output, so any of
  them can be wired straight to an oscillator's Table pin. A table with one frame is a plain
  wave. A table with several frames, wired straight in, plays its first frame. Put a Wavetable
  node in between to move along the frames.
  - **Sine, Saw, Square and Triangle** are one frame tables of the four plain waves, with no
    knobs.
  - *Frame nodes* make the frames. **Curve frame** is one cycle you draw by dragging points in its
    viewer, as with an LFO's custom wave. **Harmonics** builds one frame from a count of
    harmonics, a Tilt for how fast the upper ones fall away, and an Odd/even lean. **Sweep** makes
    many frames at once (2 to 64) as one thing changes from the first to the last: Harmonics
    building up, PWM narrowing, a Sync ratio rising, or Fold deepening. **Wave file** cuts an
    audio file into frames, either one pitch period each (Cycles) or a fixed 1024, 2048 or 4096
    samples, lines each frame up to start at a rising zero crossing so frames blend cleanly, and
    says how many frames it made when it loads. Their knobs have no pins, since a frame is worked
    out when the patch is built.
  - **Wavetable** collects frames. It has a Frame pin for each connected input and one empty pin
    below the last, up to eight. A ramp bar shows one colored region per connected input, in the
    color of its pin, with the pin's number on it. The handles between regions set how much of the
    table each frame input gets ([how to drag them](shortcuts.md#wavetable-ramp)). A region with
    several frames plays through them, and a one-frame region holds still. The Blend dropdown
    (Linear, Smooth, Step) and the Blend width knob set how neighbouring regions meet. Position,
    which a wire can drive, moves along the whole ramp, and a green dot on the bar shows where it
    is.
  - Wire a table node's output to the **Table pin on an oscillator's viewer row**. Warp, unison,
    detune and the rest still apply to it. The oscillator's viewer draws the table's wave at the
    current Position. A Wavetable can also feed another Wavetable. One table node can feed several
    oscillators, which then share its Position.
- **Sampler.** Plays back a loaded audio file, one voice per held note, pitch-shifted from the note
  relative to a Root key. The Load sample row opens a file picker; it shows the loaded file's name
  once one is chosen. WAV, AIFF, FLAC, MP3, Ogg Vorbis, and AAC/M4A load, mono or stereo. Drag an
  audio file from Finder, or a clip from a DAW's timeline, onto the editor and pick Sampler in the
  chooser that opens, and a Sampler appears there with the file already loaded. Pitch tracking, on
  by default, shifts playback
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
  wire moves its handle. Each stage's width follows
  its own knob only, so the time axis is fixed and a long release shows as a long tail. The
  LFO runs free or locks to the host's tempo. Its Wave list has a Custom wave, a shape you draw as
  a list of points in the viewer ([how to edit them](shortcuts.md#viewers)). A curve holds up to
  16 points, and the two end points only move up and down. The points are saved with the patch and with presets.
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
  The Span knob says how many units the full 0 to 1 range stands for, and A and B read and take
  numbers in those units. At the default of 100 they are percents. To add exactly 5 semitones to a
  signal that drives a Crs knob, whose range is 24 semitones wide, set Span to 24, B to 5, and the
  op to Add. Span only changes the numbers, not the signal, and it has no pin.
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
  notch. A cut and a notch have no gain, so their points move sideways only. The Analyzer checkbox
  under the graph draws the spectrum of the EQ's output behind the curve.
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

**Knobs.** Drag a knob to turn it, double click to type a value, and Option (Alt) and click to
reset it. See [Shortcuts and gestures](shortcuts.md#knobs).

**Viewers.** The oscillator, envelope, LFO and filter each draw a small picture of their shape: the
waveform, the ADSR curve, and the filter's response from 20 Hz to 20 kHz. The picture follows the
knobs. It also follows a wire that drives a knob, so a filter with an LFO on its cutoff shows the
curve sweeping. While a note plays, the envelope and LFO viewers put an orange dot on the curve
where the signal is now. The envelope dot follows the newest note, and it rests in the middle of
the sustain hold. A noise LFO has no dot. A one shot LFO's dot stops at the end of the curve.

## Editing

Add nodes from the burger menu or the search box, wire them by dragging between pins, and move,
select, splice, fold, mute and delete them with the mouse and a few keys. The strip at the bottom
plays notes, in the plugin and in the standalone app. Every gesture and key is in
[Shortcuts and gestures](shortcuts.md).

## Editor modes

The editor is in Keyboard mode, where the computer keyboard plays notes, or Edit mode, where the
same keys edit the graph. A badge above the strip at the bottom right shows which, and `Tab`
switches. Which keys do what in each mode is in [Shortcuts and gestures](shortcuts.md#editor-modes).

**Presets.** The Presets row of the burger menu saves and loads whole patches, sorted into
categories. See [Presets](presets.md).
