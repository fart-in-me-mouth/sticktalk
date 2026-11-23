// Navigation behavior that must not wait for a render: Ctrl/Cmd+K, Quick
// Switcher filtering, search filter suggestions, rail drag and drop, and the
// right-click / long-press / Shift+F10 menus for servers, folders and
// conversations. It reads only what the page shows; no message data lives here.
const $ = (s) => document.querySelector(s);
let emit = () => {};
const FILTERS = [
  ['from:', 'A person who sent it'],
  ['mentions:', 'A person it mentions'],
  ['in:', 'A channel or conversation'],
  ['has:', 'link, file, image or video'],
  ['before:', 'A date (YYYY-MM-DD)'],
  ['after:', 'A date (YYYY-MM-DD)'],
  ['during:', 'A day (YYYY-MM-DD)'],
];
const HAS = ['link', 'file', 'image', 'video'];
const day = (offset) => {
  const d = new Date(Date.now() - offset * 86400000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};

// ----- Quick Switcher -------------------------------------------------------
let switcherSelected = null;
/** How well `query` matches `text`: higher is better, -1 for no match. */
export function fuzzy(query, text) {
  if (!query) return 0;
  const at = text.indexOf(query);
  if (at === 0) return 1000 - text.length;
  if (at > 0) return (text[at - 1] === ' ' ? 800 : 600) - at;
  // Letters in order, with a bonus for each that starts a word.
  let score = 300, i = 0;
  for (let j = 0; j < text.length && i < query.length; j++) {
    if (text[j] === query[i]) { score += j === 0 || text[j - 1] === ' ' ? 8 : 1; i++; }
    else score -= 1;
  }
  return i === query.length ? Math.max(1, score) : -1;
}
const SIGILS = { '#': 'channel', '@': 'person', '*': 'server' };
function filterSwitcher() {
  const input = $('[data-nav-switcher-input]'), list = $('#nav-switcher-list');
  if (!input || !list) return;
  let query = input.value.trim().toLowerCase(), kind = null;
  if (SIGILS[query[0]]) { kind = SIGILS[query[0]]; query = query.slice(1).trim(); }
  const rows = [...list.querySelectorAll('[role=option]')];
  const scored = [];
  for (const row of rows) {
    const k = row.dataset.kind;
    const ok = !kind || k === kind || (kind === 'person' && k === 'dm');
    const score = ok ? fuzzy(query, row.dataset.search) : -1;
    if (score < 0) continue;
    // Unread and mentioned conversations come first, as the app ranks them.
    scored.push([score + Number(row.dataset.rank) * (query ? 150 : 10000), row]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  const shown = new Set(scored.slice(0, 40).map(([, row]) => row));
  for (const row of rows) row.hidden = !shown.has(row);
  // Reorder only when needed: every DOM move is a mutation the app reacts to.
  const order = scored.slice(0, 40).map(([, row]) => row);
  const current = [...list.querySelectorAll('[role=option]:not([hidden])')];
  if (order.some((row, i) => current[i] !== row)) for (const row of order) list.append(row);
  $('.nav-switch-empty')?.toggleAttribute('hidden', shown.size > 0);
  const keep = scored.find(([, r]) => r.dataset.live === switcherSelected)?.[1];
  select(keep ?? scored[0]?.[1] ?? null);
}
function select(row) {
  const list = $('#nav-switcher-list'), input = $('[data-nav-switcher-input]');
  for (const r of list?.querySelectorAll('[aria-selected=true]') ?? []) r.setAttribute('aria-selected', 'false');
  switcherSelected = row?.dataset.live ?? null;
  if (row) { row.setAttribute('aria-selected', 'true'); row.scrollIntoView({ block: 'nearest' }); input?.setAttribute('aria-activedescendant', row.id); }
  else input?.removeAttribute('aria-activedescendant');
}
function switcherKey(e) {
  const rows = [...document.querySelectorAll('#nav-switcher-list [role=option]:not([hidden])')];
  if (!rows.length) return;
  const at = Math.max(0, rows.findIndex((r) => r.getAttribute('aria-selected') === 'true'));
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    select(rows[(at + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length]);
  } else if (e.key === 'Enter') {
    e.preventDefault();
    rows[at].click();
  }
}

// ----- Search box and filter suggestions -----------------------------------
let suggestions = [], suggestIndex = -1, suggestFor = null;
function token(input) {
  const caret = input.selectionStart ?? input.value.length;
  const before = input.value.slice(0, caret);
  const start = before.search(/\S*$/);
  return { start, end: caret, text: before.slice(start) };
}
function compute(input) {
  const { text } = token(input);
  let data = { users: [], channels: [] };
  try { data = JSON.parse(input.dataset.suggest || '{}'); } catch { /* keep empty */ }
  const colon = text.indexOf(':');
  if (colon < 0) {
    return FILTERS.filter(([key]) => key.startsWith(text.toLowerCase())).map(([key, hint]) => ({ insert: key, label: key, hint, open: true }));
  }
  const key = text.slice(0, colon + 1).toLowerCase(), part = text.slice(colon + 1).toLowerCase();
  if (key === 'from:' || key === 'mentions:') {
    return (data.users ?? []).filter(([u, d]) => u.includes(part) || String(d).toLowerCase().includes(part))
      .slice(0, 8).map(([u, d]) => ({ insert: key + u, label: d, hint: '@' + u }));
  }
  if (key === 'in:') {
    const names = [...(data.channels ?? []), ...(data.users ?? []).map(([u]) => u)];
    return names.filter((n) => n.toLowerCase().includes(part)).slice(0, 8).map((n) => ({ insert: key + n, label: key + n, hint: '' }));
  }
  if (key === 'has:') return HAS.filter((h) => h.startsWith(part)).map((h) => ({ insert: key + h, label: key + h, hint: '' }));
  if (key === 'before:' || key === 'after:' || key === 'during:') {
    return [[day(0), 'Today'], [day(1), 'Yesterday'], [day(7), 'A week ago']].filter(([d]) => d.startsWith(part))
      .map(([d, hint]) => ({ insert: key + d, label: key + d, hint }));
  }
  return [];
}
function renderSuggestions(input) {
  const box = input.closest('[data-nav-search-form]')?.querySelector('.nav-suggest');
  if (!box) return;
  suggestions = document.activeElement === input ? compute(input) : [];
  suggestFor = input.id;
  suggestIndex = Math.min(suggestIndex, suggestions.length - 1);
  box.replaceChildren(...suggestions.map((s, i) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.id = `nav-suggest-${i}`;
    b.className = 'flex h-9 w-full items-center gap-2 rounded-sm px-2 text-left text-sm aria-selected:bg-secondary touch:min-h-11';
    b.setAttribute('role', 'option');
    b.setAttribute('aria-selected', String(i === suggestIndex));
    b.dataset.navSuggest = String(i);
    const label = document.createElement('span'); label.className = 'min-w-0 flex-1 truncate'; label.textContent = s.label;
    const hint = document.createElement('small'); hint.className = 'truncate text-xs text-muted-foreground'; hint.textContent = s.hint;
    b.append(label, hint);
    return b;
  }));
  const open = suggestions.length > 0;
  box.classList.toggle('hidden', !open);
  input.setAttribute('aria-expanded', String(open));
  if (open && suggestIndex >= 0) input.setAttribute('aria-activedescendant', `nav-suggest-${suggestIndex}`);
  else input.removeAttribute('aria-activedescendant');
}
function apply(input, s) {
  const { start, end } = token(input);
  const tail = s.open ? '' : ' ';
  input.value = input.value.slice(0, start) + s.insert + tail + input.value.slice(end).replace(/^\S*/, '');
  const caret = start + s.insert.length + tail.length;
  input.setSelectionRange(caret, caret);
  suggestIndex = -1;
  renderSuggestions(input);
}
function searchKey(e) {
  const input = e.target;
  const open = suggestions.length > 0 && suggestFor === input.id;
  if (open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    e.preventDefault();
    suggestIndex = (suggestIndex + (e.key === 'ArrowDown' ? 1 : -1) + suggestions.length + 1) % (suggestions.length + 1);
    if (suggestIndex === suggestions.length) suggestIndex = -1;
    renderSuggestions(input);
  } else if (open && e.key === 'Tab' && suggestions.length) {
    e.preventDefault();
    apply(input, suggestions[Math.max(0, suggestIndex)]);
  } else if (e.key === 'Enter') {
    e.preventDefault();
    if (open && suggestIndex >= 0) { apply(input, suggestions[suggestIndex]); return; }
    suggestions = [];
    renderSuggestions(input);
    emit(`nav-search:${input.value}`);
  } else if (e.key === 'Escape') {
    e.stopPropagation();
    if (open) { suggestions = []; input.closest('[data-nav-search-form]')?.querySelector('.nav-suggest')?.classList.add('hidden'); }
    else if (input.value) { input.value = ''; }
    else emit('nav-search-close');
  }
}

// ----- Rail drag and drop ---------------------------------------------------
let dragging = null;
const DROP = { before: 'shadow-[0_-4px_0_0_var(--color-primary)]', after: 'shadow-[0_4px_0_0_var(--color-primary)]', into: 'ring-2 ring-primary' };
function clearDrop() {
  for (const e of document.querySelectorAll('[data-drop]')) { e.classList.remove(...Object.values(DROP).flatMap((c) => c.split(' '))); delete e.dataset.drop; }
}
function place(target, e) {
  const r = target.getBoundingClientRect(), y = (e.clientY - r.top) / r.height;
  return y < 0.3 ? 'before' : y > 0.7 ? 'after' : 'into';
}

// ----- Menus ------------------------------------------------------------------
function menuFor(element) {
  const rail = element.closest?.('[data-rail]');
  if (rail) return rail.dataset.rail[0] === 'g' ? `nav-menu:server:${rail.dataset.rail.slice(1)}` : `nav-menu:folder:${rail.dataset.rail.slice(1)}`;
  const row = element.closest?.('.channel-row');
  const options = row?.querySelector('[data-live^="nav-menu:conv:"]');
  return options?.dataset.live ?? null;
}
let press = null, pressed = 0;

// ----- Keeping typed text across renders --------------------------------------
let held = [], hadResults = false, hadSwitcher = false, rendered = false;
export function navBeforeRender() {
  rendered = true;
  held = [...document.querySelectorAll('[data-nav-search]')].map((e) => [e.id, e.value, e === document.activeElement, e.selectionStart, e.selectionEnd]);
}
export function navRestore() {
  // Only after an app render; this module's own DOM edits also reach here.
  if (!rendered) return;
  rendered = false;
  for (const [id, value, focused, start, end] of held) {
    const e = document.getElementById(id);
    if (!e) continue;
    e.value = value;
    if (focused && document.activeElement !== e) { e.focus({ preventScroll: true }); try { e.setSelectionRange(start, end); } catch { /* not a text field */ } }
  }
  held = [];
  const results = !!$('.nav-results');
  if (results && !hadResults) {
    const phone = innerWidth < 768;
    const input = document.getElementById(phone ? 'nav-search-mobile' : 'nav-search-input');
    if (input && document.activeElement !== input) input.focus({ preventScroll: true });
  }
  hadResults = results;
  const switcher = !!$('[data-nav-switcher-input]');
  if (switcher) {
    if (!hadSwitcher) { switcherSelected = null; $('[data-nav-switcher-input]').focus(); }
    filterSwitcher();
  }
  hadSwitcher = switcher;
}

export function installNav(send) {
  emit = send;
  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      emit($('[data-nav-switcher-input]') ? 'close' : 'nav-switcher');
      return;
    }
    if (e.target.matches?.('[data-nav-switcher-input]')) { switcherKey(e); return; }
    if (e.target.matches?.('[data-nav-search]')) { searchKey(e); return; }
    if ((e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && !e.target.closest?.('.message')) {
      const action = menuFor(e.target);
      if (action) { e.preventDefault(); emit(action); }
    }
  }, true);
  document.addEventListener('input', (e) => {
    if (e.target.matches('[data-nav-switcher-input]')) { switcherSelected = null; filterSwitcher(); }
    if (e.target.matches('[data-nav-search]')) { suggestIndex = -1; renderSuggestions(e.target); }
  });
  document.addEventListener('focusin', (e) => { if (e.target.matches?.('[data-nav-search]')) renderSuggestions(e.target); });
  document.addEventListener('focusout', (e) => {
    if (!e.target.matches?.('[data-nav-search]')) return;
    const form = e.target.closest('[data-nav-search-form]');
    setTimeout(() => { if (!form?.contains(document.activeElement)) form?.querySelector('.nav-suggest')?.classList.add('hidden'); }, 0);
  });
  // Pressing a suggestion must not take focus from the box first.
  document.addEventListener('mousedown', (e) => { if (e.target.closest('[data-nav-suggest]')) e.preventDefault(); });
  document.addEventListener('click', (e) => {
    if (performance.now() - pressed < 600 && menuFor(e.target)) { e.preventDefault(); e.stopPropagation(); return; }
    const s = e.target.closest('[data-nav-suggest]');
    if (s) {
      const input = s.closest('[data-nav-search-form]')?.querySelector('[data-nav-search]');
      const pick = suggestions[Number(s.dataset.navSuggest)];
      if (input && pick) { input.focus(); apply(input, pick); }
      return;
    }
    const swatch = e.target.closest('[data-nav-color]');
    if (swatch) { const field = $('#nav-folder-color'); if (field) field.value = swatch.dataset.navColor; }
  }, true);
  document.addEventListener('submit', (e) => {
    const form = e.target.closest('[data-nav-form]');
    if (form) { e.preventDefault(); emit(`nav-form:${form.dataset.navForm}`); }
    if (e.target.matches('[data-nav-search-form]')) e.preventDefault();
  });
  document.addEventListener('change', (e) => {
    if (e.target.dataset.navFavCategory) emit(`nav-fav-move:${e.target.dataset.navFavCategory},${e.target.value}`);
  });
  document.addEventListener('contextmenu', (e) => {
    if (e.target.closest('.message,.live-modal')) return;
    const action = menuFor(e.target);
    if (action) { e.preventDefault(); emit(action); }
  });
  document.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch') return;
    const action = menuFor(e.target);
    if (!action || e.target.closest('.live-modal')) return;
    press = { x: e.clientX, y: e.clientY, timer: setTimeout(() => { press = null; pressed = performance.now(); emit(action); }, 500) };
  });
  const cancel = () => { if (press) clearTimeout(press.timer); press = null; };
  document.addEventListener('pointermove', (e) => { if (press && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 12) cancel(); });
  document.addEventListener('pointerup', cancel);
  document.addEventListener('pointercancel', cancel);
  document.addEventListener('dragstart', (e) => {
    const item = e.target.closest?.('[data-rail]');
    if (!item) return;
    dragging = item.dataset.rail;
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', dragging);
  });
  document.addEventListener('dragover', (e) => {
    const target = dragging && e.target.closest?.('[data-rail]');
    if (!target) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    const where = target.dataset.rail === dragging ? null : place(target, e);
    if (target.dataset.drop === where) return;
    clearDrop();
    if (where) { target.dataset.drop = where; target.classList.add(...DROP[where].split(' ')); }
  });
  document.addEventListener('drop', (e) => {
    const target = dragging && e.target.closest?.('[data-rail]');
    const where = target?.dataset.drop;
    clearDrop();
    if (target && where) { e.preventDefault(); emit(`nav-rail-drop:${dragging},${target.dataset.rail},${where}`); }
    dragging = null;
  });
  document.addEventListener('dragend', () => { clearDrop(); dragging = null; });
}
