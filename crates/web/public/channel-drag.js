// Channel drag and drop in a server's channel list (docs/features/channel-drag.md),
// loaded by nav.js when a manager first presses a row. Rows carry
// `data-chan="<id>,<parent id or 0>,<c|f|t>"` (category, forum, other) and
// `draggable` only for people who may manage channels. A drop lands
// in a parent just before one of its children, or after the last child of its
// own sort: at every level the channels are listed before the categories. The
// app checks the move again (`channel_order.rs`) and the server has the last word.
let marked = null, scroller = null;
const CHAN_LINE = ['relative', 'before:pointer-events-none', 'before:absolute', 'before:right-0', 'before:left-(--chan-drop-x)', 'before:z-10', 'before:h-0.5', 'before:rounded-full', 'before:bg-primary'];
const CHAN_EDGE = { top: 'before:-top-px', bottom: 'before:-bottom-px' };
const CHAN_INTO = ['rounded-md', 'ring-2', 'ring-primary', 'ring-inset', 'bg-sidebar-accent'];
const chanRow = (el) => {
  const [id, parent, kind] = el.dataset.chan.split(',');
  return { el, id, parent, kind, cat: kind === 'c', depth: Number(el.dataset.depth) || 0 };
};
const chanRows = () => [...document.querySelectorAll('.channel-scroll [data-chan]')].map(chanRow);
function unmark() {
  if (marked) { marked.el.classList.remove(...marked.added); marked.el.style.removeProperty('--chan-drop-x'); delete marked.el.dataset.chanDrop; }
  marked = null;
}
function mark(el, classes, where, x = 0) {
  unmark();
  const added = classes.filter((c) => !el.classList.contains(c));
  el.classList.add(...added);
  el.style.setProperty('--chan-drop-x', `${x}px`);
  el.dataset.chanDrop = where;
  marked = { el, added };
}
/** Where the dragged channel `x` lands for a pointer over `target` (`edge`:
 * before, after, into, or `end` for the empty space under the list), as
 * { parent, before, show, depth }, or null when it cannot go there or would not move. */
function chanLanding(rows, x, target, edge) {
  const from = rows.findIndex((r) => r.id === x.id);
  if (from < 0) return null;
  let to = from + 1;
  while (to < rows.length && rows[to].depth > rows[from].depth) to++;
  const inside = new Set(rows.slice(from, to).map((r) => r.id));
  if (target && inside.has(target.id)) return null;
  const rest = rows.filter((r) => !inside.has(r.id));
  const kids = (p) => rest.filter((r) => r.parent === p);
  const last = (r) => { let i = rest.indexOf(r); while (i + 1 < rest.length && rest[i + 1].depth > r.depth) i++; return rest[i]; };
  let parent, before;
  if (edge === 'end') { parent = '0'; before = '0'; }
  else if (edge === 'into' || (edge === 'after' && target.kind !== 'f' && kids(target.id).length)) {
    // The bottom edge of a row with channels inside it is the top of that list.
    parent = target.id;
    before = edge === 'into' ? '0' : (kids(parent).find((r) => r.cat === x.cat)?.id ?? '0');
  } else {
    parent = target.parent;
    const same = kids(parent).filter((r) => r.cat === x.cat);
    if (target.cat !== x.cat) before = x.cat ? (same[0]?.id ?? '0') : '0';
    else before = edge === 'before' ? target.id : (same[same.indexOf(target) + 1]?.id ?? '0');
  }
  // Where it is now changes nothing.
  const now = rows.filter((r) => r.parent === x.parent && r.cat === x.cat);
  if (parent === x.parent && before === (now[now.indexOf(rows[from]) + 1]?.id ?? '0')) return null;
  // The line goes at the indentation of the level it joins.
  const holder = rest.find((r) => r.id === parent);
  if (parent !== '0' && !holder) return null;
  const depth = holder ? holder.depth + 1 : 0;
  let show;
  if (edge === 'into') show = { row: target, edge: 'into' };
  else if (before !== '0') show = { row: rest.find((r) => r.id === before), edge: 'top' };
  else {
    const all = kids(parent), same = all.filter((r) => r.cat === x.cat), cats = all.filter((r) => r.cat);
    if (same.length) show = { row: last(same.at(-1)), edge: 'bottom' };
    else if (!x.cat && cats.length) show = { row: cats[0], edge: 'top' };
    else if (all.length) show = { row: last(all.at(-1)), edge: 'bottom' };
    else show = { row: holder, edge: 'bottom' };
  }
  if (!show.row) return null;
  return { parent, before, show, depth };
}
function chanShow(at) {
  if (!at) { unmark(); return; }
  const { row, edge } = at.show;
  const where = edge === 'into' ? 'into' : `${edge}${at.depth}`;
  if (marked?.el === row.el && row.el.dataset.chanDrop === where) return;
  if (edge === 'into') mark(row.el, CHAN_INTO, where);
  else mark(row.el, [...CHAN_LINE, CHAN_EDGE[edge]], where, (Math.min(at.depth, 8) - Math.min(row.depth, 8)) * 12);
}
/** Near the top or bottom of the list, scroll it; only while a drag is over that edge. */
function chanScroll(list, y) {
  let speed = 0;
  if (list) {
    const r = list.getBoundingClientRect(), zone = Math.min(48, r.height / 4);
    if (y < r.top + zone) speed = -Math.ceil(((r.top + zone - y) / zone) * 14);
    else if (y > r.bottom - zone) speed = Math.ceil(((y - r.bottom + zone) / zone) * 14);
  }
  if (!speed) { if (scroller) cancelAnimationFrame(scroller.frame); scroller = null; return; }
  if (scroller) { scroller.list = list; scroller.speed = speed; return; }
  const step = () => {
    const top = scroller.list.scrollTop;
    scroller.list.scrollTop += scroller.speed;
    if (scroller.list.scrollTop === top) { scroller = null; return; }
    scroller.frame = requestAnimationFrame(step);
  };
  scroller = { list, speed, frame: requestAnimationFrame(step) };
}
/** The drag is over: no marker, no scrolling. */
export function end() { unmark(); chanScroll(null); }
/** A dragover while `chan` (the row being dragged) is dragged: mark where it would land in `chan.at`. */
export function over(e, chan) {
  const list = e.target.closest?.('.channel-scroll');
  if (!list) { unmark(); chanScroll(null); chan.at = null; return; }
  e.preventDefault();
  chanScroll(list, e.clientY);
  const rows = chanRows(), el = e.target.closest('[data-chan]');
  let at = chan.at;
  if (el) {
    const target = chanRow(el), r = el.getBoundingClientRect(), y = (e.clientY - r.top) / r.height;
    const edge = target.kind === 'f' ? (y < 0.5 ? 'before' : 'after') : y < 0.3 ? 'before' : y > 0.7 ? 'after' : 'into';
    at = chanLanding(rows, chan, target, edge);
  } else if (rows.length && e.clientY > rows.at(-1).el.getBoundingClientRect().bottom) {
    at = chanLanding(rows, chan, null, 'end');
  }
  // The list was drawn again under the pointer: wait for the next row.
  if (at && !at.show.row.el.isConnected) at = null;
  chan.at = at;
  e.dataTransfer.dropEffect = at ? 'move' : 'none';
  chanShow(at);
}
