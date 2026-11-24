# Channel drag and drop (task `chandrag`)

People who may manage channels drag text, voice, stage, forum and announcement
channels and categories in a server's channel list to a new place. No protocol
change: a drop sends the existing `Command::ReorderChannels` with every channel
of the server, and everyone gets the existing `Event::ChannelsReordered`.

Code: `crates/web/src/channel_order.rs` (where a move lands, the placement
list, native tests), `crates/web/src/live/channel_drag.rs` (the drop, the
optimistic update, the snap back), `crates/web/public/channel-drag.js` (landing
rules in the DOM, drop marker, edge scrolling), the stub in
`crates/web/public/nav.js`, the server checks in `crates/server/src/commands.rs`
(`ReorderChannels`), tests in `crates/server/src/launch_tests.rs`
(`reordering_checks_membership_overwrites_servers_and_loops`) and
`crates/web/tests/channel-drag.mjs`.

## Who can drag

- Rows get `draggable` and `data-chan` only when the person is the owner, an
  administrator or has Manage Channels server-wide: the same check as the Move
  Channel page and the server's own first check. Everyone else's list renders
  exactly as before (no attribute, no grab, no listener work).
- Channel overwrites are known only to the server. A manager whom an overwrite
  denies Manage Channels on one channel can still start dragging it; the
  server refuses, and the list snaps back with a message.

## Where a drop lands

At every level the sidebar lists channels before categories, so a channel
always lands among the channels and a category among the categories.

- **Top 30% of a row:** just before it, at its level. Over a row of the other
  sort, the nearest allowed place at that level (a channel: after the last
  channel, which is just above the first category; a category: before the first
  category).
- **Bottom 30% of a row:** just after it. If the row has channels inside it,
  the bottom edge is the top of that inner list (as in file trees).
- **Middle of a row:** inside it, after the last channel (or category) of its
  own sort there. Not on a forum (it holds posts): a forum's row is split in
  halves, before and after.
- **The empty space under the list:** the end of the top level.
- Not into itself or anything inside it (no marker), and a place where it
  already is shows no marker and sends nothing.
- Everything inside a moved channel moves with it.
- Categories here never collapse, so there is nothing hidden to expand on hover.

The app checks the landing again (`channel_order::moved`: same server, not a
thread, not into itself or a forum, the `before` sibling of the right sort) and
renumbers only the set it lands in and the set it leaves, from 0, in the order
shown. Other sets keep their positions. The Move Channel page now uses the same
function (it also leaves threads out of the list, which it used to include).

## Marker and scrolling

- A 2px `primary` line on the top edge of the row it goes before, or the bottom
  edge of the last row it goes after, starting at the indentation of the level
  it joins; "inside" is an inset `primary` ring and the hover background on the
  row. Same colours as the rail's drop marker.
- Within 48 px of the list's top or bottom edge the list scrolls (one
  `requestAnimationFrame` loop, only while a drag is over that edge, stopped at
  the end of the list or when the drag leaves the edge).

## Optimistic, then the server decides

The drop applies the new placements to the local model at once and sends the
command. On an error answer every channel the move changed that nothing has
moved since goes back, and the toast says, in plain words, "You don’t have
permission to move that channel there, so it’s back where it was." (Forbidden)
or "That channel couldn’t be moved there, so it’s back where it was." Other
members see the change through `ChannelsReordered`, without reloading. Up to
16 moves can wait for an answer at once.

## Server checks (`ReorderChannels`)

Already there before this task: Manage Channels server-wide; the list must
hold exactly the channels the actor sees (so every one is this server's);
unique IDs; a parent must be a channel of this server and not a thread or
forum; positions that change must be dense; no channel inside itself, however
deep, counting channels the actor cannot see.

Changed by this task:

- Channel overwrites are checked for what **moves**: each channel whose place
  changes needs Manage Channels on it, and when its parent changes, on the new
  parent and on the old one. Before, every channel in the list needed it, so one
  channel locked by an overwrite stopped the whole server's list from being
  arranged, while taking a channel out of a parent was not checked against that
  parent.
- The parent is compared with where the channel **really** is. A channel
  shown at the top because its real parent is hidden from the actor used to be
  pulled out of that parent when the top level was renumbered; now that counts
  as leaving the hidden parent, which the actor cannot manage, and is refused.

Tests: a member without the permission, a channel or parent from another
server, a loop through two channels, a channel locked by an overwrite (cannot
move, cannot receive, does not block other moves), leaving a parent the actor
may not manage, and the hidden-parent case.

## Keyboard, screen readers and touch

- **Move Channel** is in a channel's options menu (the "…" button,
  right-click, long-press, Shift+F10) for people who manage channels, and in
  Server Settings → Channels → Edit for channels and categories. It is the
  keyboard and screen-reader path.
- **Touch screens:** no drag. A press on a row with a touch pointer turns the
  row's `draggable` off until the finger lifts, so a long press keeps opening
  the menu (with Move Channel). HTML drag and drop on touch is uneven across
  mobile browsers, and pointer-event dragging would fight the list's scrolling
  and the long-press menu; phones use the menu.

## Drops elsewhere

The drag carries only `application/x-sticktalk-channel`: no text and no files.
Dropped on the message box it types nothing; over the conversation the file
drop overlay does not appear (it reacts only to `Files`). The drag does not
select text or open the channel (no click follows a drag).

## Cost

- `channel-drag.js` loads when a manager first presses a row with a mouse or
  pen, and the WebAssembly drop handler (`drop_channel`, its own split module)
  when a drag starts. Nobody else downloads either.
- `nav.js` (first screen) grows by the stub only; no new listeners: the
  existing delegated `dragstart`/`dragover`/`drop`/`dragend`/`pointerdown`
  listeners branch on a channel drag.
