# Presets

Presets are files in `~/Library/Application Support/Flode/Presets`, one per name. Each holds the
graph and the four macro values. A category is a folder inside that one, so `Bass/Reese.flode` is
the Reese preset in the Bass category.

The Presets row of the burger menu opens a list with "Save as..." and "Init", then the categories,
then any presets with none. "Init" goes back to the starting patch and resets the macros, and it
does not ask about unsaved changes. A category opens a submenu of its presets when you hover it or
click it.

"Save as..." opens the system save dialog. It starts in the preset folder, or in the current
preset's category folder, and suggests the current name. The file name is the preset's name, and
Flode adds `.flode` to it if you leave that off. A folder directly inside the preset folder is the
category, so saving into `Bass` files the preset under Bass in the list, and a new folder makes a
new category. A file saved directly in the preset folder has no category. A file saved anywhere
else is written where you chose, and it works, but it does not appear in the list. Saving over an
existing file replaces that preset. Canceling the dialog does nothing.

![The presets menu with the Pad submenu open](images/presets-menu.png)

Twelve factory presets are built in code in `src/editor/factory/`: Sub Bass, Reese, Wobble Riot,
and Hollow Sub in Bass; Pulse Lead, Bell Lead, Arp Pluck, Chord Stab, and Cascade in Lead; Warm
Pad, Dusk Pad, and Chord Swell in Pad. To write them into your preset folder:

```sh
cargo test --lib write_factory_presets -- --ignored
```
