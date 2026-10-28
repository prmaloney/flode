# Status

Flode is young. The DSP is covered by tests for level, stability, filter and delay behavior, and
allocation on the audio thread, but how it sounds is still being tuned by ear. Known limits, grouped
by theme:

**Per-patch slot counts.** A patch is built from a fixed number of slots for each kind of thing it
can hold, counted in the order the nodes were added. Going past a count does not crash; the extra
node is dimmed (for LFOs and envelopes), passes its input straight through (for a fifth effect,
filter, or Gain after an effect), or simply makes no signal (for maps, math nodes, and note
sources), depending on the kind:

- Four instances of each effect kind, four filters that run after an effect, and four Gain nodes
  that run after an effect. A fifth passes its input through. The same goes for monitors, which
  show nothing past the fourth, though a monitor only costs one stage of its slot's 16. If the
  monitors push a slot past that limit, the slot runs without them rather than going silent.
- 8 LFOs and 8 envelopes. One past that is dimmed and tagged Over limit, and makes no signal.
- 8 note sources (MIDI inputs, arpeggiators, and chords sharing the one count) together, since
  notes travel per wire rather than as one broadcast: an oscillator or a sampler reads the slot its
  Notes pin is wired to. Deleting an earlier node can reassign a later one's slot, since nodes have
  no id of their own that survives an edit, the same caveat the LFO and envelope counts above have.
- 24 map and math nodes together. One past that makes no signal, so a knob wired to it is not
  driven. A map with nothing wired to its input makes no signal either, and a loop of maps makes
  none. A loop of math nodes is cut where it closes, and the node that would read itself uses its
  own knob instead.
- A group counts everything inside it toward these same limits: LFOs, envelopes, the effect pools,
  and the stages in a slot. A group copied three times counts its nodes three times.

**Latency and timing.** The distortion adds about 15 samples of latency when its mix is above zero.
Parallel chains are not delay compensated, so a distorted chain merged with a clean one can comb
filter a little. The FM mix delays the signal by a few cycles of the note, so that knob moves do
not bend the pitch.

**Audio Unit.** The Audio Unit is the CLAP plugin wrapped by clap-wrapper, and it passes `auval`. It
has not been tried in Logic Pro itself. Keyboard mode does not play notes in GarageBand, which takes
the computer keyboard first (see [Hosts](shortcuts.md#hosts)). The bundle is signed ad hoc and not
notarized, so a copy that came through a download needs its quarantine flag cleared.

**Groups.** The Output and Macro nodes cannot go inside a group, and neither can a group's own edge
nodes. A group node cannot be added from the add menu or the search box. Make one with `Shift+G`.

**Sampler.** Dropping a file on the editor is built for macOS, where it has been checked only
against files; a drag straight from a DAW timeline depends on what the DAW puts on the drag, and
Windows has the hook but has not been tried. A preset or a saved group that names a sample file on a
path that does not exist on another machine loses that sample silently when it loads, reported in
the status line rather than failing the whole patch, until you load the file again by hand.

**Presets.** The preset list has no scrolling, delete, or rename. Delete the files to remove a
preset.
