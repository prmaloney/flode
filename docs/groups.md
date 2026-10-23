# Groups

A group is a node that holds a graph of its own. Select some nodes and press `Shift+G` to turn
them into one. Each wire that crossed the edge of the selection goes through a pin on the group.
`Shift+Alt+G` breaks the selected group back into its nodes and joins the wires that ran through
its pins. Ableton Live takes `Cmd+G` and `Cmd+Shift+G` for its track groups, which is why these
are not the keys. They also work in the standalone app and in hosts that pass them on, as `Cmd+G`
and `Cmd+Shift+G` (`Ctrl` on Windows and Linux). Both are also in the right-click menu, where a
group node offers Open, Rename, Bypass, Ungroup, Duplicate, Save group and Delete, and a selection
of several nodes offers Group.

- Double clicking a group's header, or clicking the box icon in it, opens it. So does Open in the
  right-click menu. `Esc` closes the open group, or closes an open menu first. The close button in
  the pane's title strip closes it too. A text box such as Rename or the search box keeps `Esc`
  for itself. `Tab` is the editor mode toggle now (see [Editor modes](nodes.md#editor-modes)), not
  a group shortcut, in every context including inside an open group.
- An open group floats in a pane over a dimmed backdrop, with its path in the title strip, such as
  `Main > E Piano > Reverb Tail`. Scroll and pinch work inside the pane. The close button, or a
  click on the dimmed margin, closes it. A drag that starts on the margin does not. Groups open
  inside groups, one pane at a time. The sound keeps playing while a group is open, and the burger
  menu and the key strip stay in use.
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

## Saving a group to a file

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
  the name. The Groups row of the burger menu lists every saved group. Either way the group follows
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
  there. The status line under the burger menu says why. The Groups list shows as many rows as
  fit above the key strip, then notes how many more are not shown. It does not scroll, the same as
  the preset list, so keep the folder short or use the search.

The Macro node can name its knobs. Right-click a row and pick Rename, and an empty name puts the
default back. The name shows on the node only. The host still lists the parameters as Macro 1 to 4.
