// Loaded on the first press of a reaction chip or when an expression picker
// first shows (public/live.js).
//
// Press and hold a reaction chip (touch or pen) to use its other kind: a Super
// Reaction on a normal chip, a normal reaction on a Super one. One timer
// exists only while a finger is down.
const HOLD_MS = 450;
export function press(chip, down, run) {
  const start = { x: down.clientX, y: down.clientY };
  let fired = false;
  const done = () => {
    clearTimeout(timer);
    chip.removeEventListener('pointerup', done);
    chip.removeEventListener('pointercancel', done);
    chip.removeEventListener('pointerleave', done);
    chip.removeEventListener('pointermove', moved);
  };
  const moved = (e) => { if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 12) done(); };
  const timer = setTimeout(() => {
    fired = true;
    done();
    // The release that follows is not also a tap on the chip.
    const swallow = (e) => { e.preventDefault(); e.stopPropagation(); };
    chip.addEventListener('click', swallow, { capture: true, once: true });
    setTimeout(() => chip.removeEventListener('click', swallow, { capture: true }), 1000);
    run(chip.dataset.liveAlt);
  }, HOLD_MS);
  chip.addEventListener('pointerup', done);
  chip.addEventListener('pointercancel', done);
  chip.addEventListener('pointerleave', done);
  chip.addEventListener('pointermove', moved);
  return () => fired;
}

// The expression picker (src/live/ui/picker.rs). Called after every render
// once this module is loaded; with no picker open it only lets go of what it
// held. While one is open: arrow keys between its tabs, the category bar
// that jumps to a section and marks the one at the top as the list scrolls,
// asking for more of a section as it comes near, and focus on the search
// field when a tab is opened by pointer. No timers.
let list = null, observer = null, asked = new WeakMap(), shown = null;
let toSearch = false, byKeys = false, pinned = null, marking = false, installed = false;
const onScroll = () => {
  if (marking) return;
  marking = true;
  requestAnimationFrame(() => { marking = false; mark(); });
};
// A person scrolling takes over from a jump.
const unpin = () => { pinned = null; };
function mark() {
  if (!list?.isConnected) return;
  const sections = [...list.querySelectorAll('[data-emoji-section]')];
  if (!sections.length) return;
  const top = list.getBoundingClientRect().top + 24;
  let at = sections[0];
  for (const s of sections) { if (s.getBoundingClientRect().top <= top) at = s; else break; }
  // A short last section never reaches the top: at the end, it is the one.
  if (list.scrollTop > 0 && list.scrollTop + list.clientHeight >= list.scrollHeight - 2) at = sections.at(-1);
  const key = pinned ?? at.dataset.emojiSection;
  for (const b of list.parentElement.querySelectorAll('[data-emoji-jump]')) {
    const on = b.dataset.emojiJump === key;
    if ((b.getAttribute('aria-current') === 'true') === on) continue;
    b.setAttribute('aria-current', String(on));
    if (on) b.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
}
function install() {
  installed = true;
  document.addEventListener('keydown', (e) => {
    const tab = e.target.closest?.('.picker-tabs [role=tab]');
    if (!tab) return;
    const tabs = [...tab.parentElement.querySelectorAll('[role=tab]')];
    const i = tabs.indexOf(tab);
    const to = { ArrowLeft: i - 1, ArrowRight: i + 1, Home: 0, End: tabs.length - 1 }[e.key];
    if (to === undefined) return;
    e.preventDefault();
    const next = tabs[(to + tabs.length) % tabs.length];
    byKeys = true;
    next.focus();
    next.click();
  });
  document.addEventListener('click', (e) => {
    const jump = e.target.closest?.('[data-emoji-jump]');
    if (!jump || !list) return;
    const key = jump.dataset.emojiJump;
    const section = [...list.querySelectorAll('[data-emoji-section]')].find((s) => s.dataset.emojiSection === key);
    if (!section) return;
    pinned = key;
    list.scrollTop = section === list.querySelector('[data-emoji-section]') ? 0
      : list.scrollTop + section.getBoundingClientRect().top - list.getBoundingClientRect().top;
    mark();
  });
}
export function picker(emit) {
  const modal = document.querySelector('.picker-modal');
  if (!modal) {
    if (list) { list.removeEventListener('scroll', onScroll); observer?.disconnect(); list = observer = null; }
    shown = pinned = null;
    return;
  }
  if (!installed) install();
  const kind = modal.dataset.dialog;
  if (kind !== shown) {
    // Opened, or another tab: the search field, unless the arrow keys are
    // moving along the tabs or a touch keyboard would cover the picker.
    toSearch = !byKeys && !matchMedia('(pointer: coarse)').matches;
    byKeys = false;
    shown = kind;
    pinned = null;
  }
  const field = toSearch && modal.querySelector('.picker-search input');
  if (field) { toSearch = false; field.focus({ preventScroll: true }); }
  const next = modal.querySelector('.picker-scroll');
  if (next !== list) {
    if (list) { list.removeEventListener('scroll', onScroll); for (const k of ['wheel', 'touchmove', 'pointerdown', 'keydown']) list.removeEventListener(k, unpin); }
    observer?.disconnect();
    list = next;
    asked = new WeakMap();
    observer = list && new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (!e.isIntersecting || !e.target.isConnected) continue;
        observer.unobserve(e.target);
        // Reached from below (after a jump): all of it, so nothing is drawn
        // above what is being looked at.
        const below = e.boundingClientRect.top < (e.rootBounds?.top ?? 0);
        emit(below ? e.target.dataset.emojiAll : e.target.dataset.emojiMore);
      }
    }, { root: list, rootMargin: '96px 0px 96px 0px' });
    if (list) {
      list.addEventListener('scroll', onScroll, { passive: true });
      for (const k of ['wheel', 'touchmove', 'pointerdown', 'keydown']) list.addEventListener(k, unpin, { passive: true });
    }
  }
  if (!list) return;
  // Each part not drawn yet is watched once per request it would make.
  for (const spacer of list.querySelectorAll('[data-emoji-more]')) {
    if (asked.get(spacer) === spacer.dataset.emojiMore) continue;
    asked.set(spacer, spacer.dataset.emojiMore);
    observer.unobserve(spacer);
    observer.observe(spacer);
  }
  mark();
}
