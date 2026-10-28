# Shortcuts and gestures

Every key and mouse gesture in the editor, in one place. The other docs link here instead of
repeating them. `Cmd` means the command key on macOS and `Ctrl` on Windows and Linux.

## Editor modes

The editor is in one of two modes, shown by a small badge above the key strip at the bottom right.
`Tab` switches between them, everywhere, including inside an open group.

- **Keyboard** (the default) plays notes from the computer keyboard.
- **Edit** turns the piano off and hands plain letter keys to the editor.

The shortcuts below say which mode they work in. A shortcut on a piano key works in both modes
only if it needs `Shift` or `Cmd`, which the piano ignores.

## Keys

| Key | Mode | What it does |
| --- | --- | --- |
| `A` `W` `S` `E` `D` `F` `T` `G` `Y` `H` `U` `J` `K` `O` `L` `P` `;` | Keyboard | Play notes, as on a piano's keys |
| `Z` / `X` | Keyboard | Move the key strip down or up one octave, up to three |
| `Tab` | Both | Switch between Keyboard and Edit mode |
| `Shift+A` | Both | Open the search box at the cursor. Type part of a node's name, pick with the arrow keys and `Enter` or a click, and the node follows the cursor until you click to place it. `Esc` or a click outside closes it |
| `Shift+D` | Both | Duplicate the selected node |
| `M` | Both | Mute a source, or bypass a processor. Also in the right-click menu |
| `Shift+G` | Both | Group the selected nodes ([Groups](groups.md)). `Cmd+G` does the same where the host passes it on |
| `Shift+Alt+G` | Both | Ungroup the selected group. `Cmd+Shift+G` does the same where the host passes it on |
| `Esc` | Both | Close the open group, or close an open menu or box first. A text box keeps `Esc` for itself |
| `G` | Edit | Grab the selection. It follows the cursor with no button held. A click or `Enter` sets it down, splicing a lone node into the wire under it. `Esc` or a right click cancels and snaps every node back |
| `X` or `Delete` | Edit | Remove the selection |
| `Cmd+X` | Edit | Remove the selection and reconnect around it (Blender's Delete and Reconnect). A node with one clear through path, such as a filter, an effect, a map, a math node or an arpeggiator, hands what fed it to what it fed. A node with no such path, such as the output, is just removed. Deleting a run of nodes bypasses the whole run. Also in the right-click menu |
| `A` | Edit | Select every node |
| `Alt+A` | Edit | Clear the selection |
| `Enter` / `Esc` | Text box | Keep or cancel what you typed in a knob value box or a rename box |

### Hosts

A plugin only sees the keys its host lets through. Ableton Live keeps `Cmd+G` and `Cmd+Shift+G` for
its track groups, which is why Flode groups with `Shift+G` and `Shift+Alt+G` first. In GarageBand
the host takes the computer keyboard first, so Keyboard mode does not play notes there. The key
strip still does, and so does a MIDI keyboard.

## Mouse and canvas

| Gesture | What it does |
| --- | --- |
| Click a node header | Select the node. Drag it to move it |
| `Shift`+click a node | Add it to the selection, or drop it from the selection |
| Drag over empty canvas | Draw a box that selects every node it touches. `Shift` keeps the current selection under it |
| Drag a selected node's header | Move the whole selection |
| Drag a node onto a wire | Splice it in. A single node only. If the two nodes on the wire are closer than the new node and a margin on each side, the node after it, and everything that node feeds, ease to the right |
| Splice a map or math node onto a wire that drives a knob | The old source feeds the map's shape pin, or input A on a math node, and the node's output drives the knob. A math node keeps its other input at its knob's number, so splicing one in changes the signal. A map at its defaults does not |
| Right-click a node | The node's menu: Delete, Bypass, Duplicate and so on. On a selection it acts on the whole group. Auto pack there lines the selection up in columns by its wires, sources on the left |
| Drag from a pin | Draw a wire. Only pins that carry the same kind of signal can join |
| Drag from an input pin with no wire, drop on an output pin | Wire the two from the input end |
| Drag from an input pin with no wire, drop on empty canvas | Open the search box with only the nodes that can feed that input. The one you pick is added, wired to it, and follows the cursor until you click to place it |
| Drag from an input that already has a wire | Detach the wire |
| `Shift` and right-drag from one node to another | Connect them. The orange line follows the cursor and the target is outlined. A pair with the same name wins, such as Shape to Shape. Otherwise the first pair whose input is free is used. If the first node has nothing to give the second, the wire goes the other way |
| Fold a section, or collapse a node | Click the section header or the node's arrow. A collapsed node keeps its pins |
| Scroll | Pan the canvas. Scrolling inside an open group pans that group |
| Pinch | Zoom |
| Click the key strip | Play a note. The strip also shows the notes the host sends |
| Octave buttons at the strip's left | Move the strip up or down an octave, up to three. `Z` and `X` do the same in Keyboard mode |
| Drop an audio file on the editor | Make a Sampler with the file loaded |

## Knobs

| Gesture | What it does |
| --- | --- |
| Drag up or down | Turn the knob |
| Double click | Type an exact value, in the units the node shows, such as Hz, dB or a percentage. `Enter` sets it, `Esc` cancels, and a click elsewhere keeps what you typed |
| `Option` (`Alt`) and click | Put the knob back to its default |

A knob with a green pin can be driven by any control wire. It then shows the live value.

## Viewers

| Gesture | What it does |
| --- | --- |
| Drag an envelope, map or EQ handle | Turn the knob or knobs the handle sets. The handle also follows a knob or a control wire that drives it |
| Double click an envelope handle | Put its knobs back to their defaults |
| Double click an EQ point | Reset that band |
| Drag a point on a Custom LFO wave or a Curve frame | Move the point. The two end points only move up and down |
| Double click empty space on a Custom LFO wave or a Curve frame | Add a point there, up to 16 |
| Double click a point on a Custom LFO wave or a Curve frame | Remove the point |

## Wavetable ramp

| Gesture | What it does |
| --- | --- |
| Drag a handle between two regions | Move share from one frame to its neighbour. The total stays the same |
| Double click the bar or a handle | Put every share back to equal |
| Hover a region or a Frame pin | Highlight the other, so you can see which pin makes which region |

## Groups

| Gesture | What it does |
| --- | --- |
| Double click a group's header, or click its box icon | Open the group. Open in the right-click menu does the same |
| Click the close button, or click the dimmed margin | Close the open group. `Esc` does too |
| Right-click a pin on a group, or on its edge nodes | Rename or remove the pin |
| Click a group's name in the pane's title | Rename it |
| `M` on a selected group, or Bypass in its menu | Switch it off. A bypassed group hands each input to the output of the same kind in the same order |

See [Groups](groups.md) for how groups work.
