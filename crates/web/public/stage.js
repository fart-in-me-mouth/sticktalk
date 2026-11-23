// A Stage's audience: a virtual list. A thousand listeners cost the same as
// twenty: only the rows on screen exist, redrawn when the list scrolls or the
// roster changes (no timer, no animation loop). Loaded with media.js.
const ROW = 44;
const OVERSCAN = 6;
let audience = [];
let me = '';
let moderator = false;
let list = null;
let spacer = null;
let top = 0;
let drawn = '';
let frame = 0;

function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.classList.add('icon');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(svg.namespaceURI, 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}
function action(label, live, kind) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `button ${kind} h-8 px-3 text-sm`;
  b.dataset.live = live;
  b.textContent = label;
  return b;
}
function row(m, index) {
  const r = document.createElement('div');
  r.className = 'stage-member';
  r.setAttribute('role', 'listitem');
  r.style.top = `${index * ROW}px`;
  r.dataset.user = m.id;
  const pic = document.createElement('span');
  pic.className = 'stage-member-avatar';
  if (m.avatar) {
    const img = document.createElement('img');
    img.alt = '';
    img.loading = 'lazy';
    img.src = m.avatar;
    img.onerror = () => { pic.textContent = m.name.slice(0, 2); };
    pic.append(img);
  } else pic.textContent = m.name.slice(0, 2);
  const name = document.createElement('span');
  name.className = 'stage-member-name';
  name.textContent = m.id === me ? `${m.name} (you)` : m.name;
  r.append(pic, name);
  if (m.hand) {
    const hand = icon('star');
    hand.classList.add('stage-member-hand');
    hand.setAttribute('aria-hidden', 'false');
    hand.setAttribute('aria-label', 'Requested to speak');
    hand.setAttribute('role', 'img');
    r.append(hand);
  }
  if (moderator && m.id !== me) {
    if (m.hand) {
      r.append(action('Accept', `stage:accept,${m.id}`, 'primary'));
    } else if (m.invited) {
      const s = document.createElement('small');
      s.className = 'stage-member-note';
      s.textContent = 'Invited';
      r.append(s);
    } else {
      const b = action('Invite to Speak', `stage:invite,${m.id}`, 'secondary');
      b.setAttribute('aria-label', `Invite ${m.name} to speak`);
      r.append(b);
    }
  }
  r.setAttribute('aria-label', `${m.name}${m.hand ? ', requested to speak' : ''}`);
  return r;
}
function draw(force) {
  if (!list) return;
  const height = list.clientHeight || 360;
  const first = Math.max(0, Math.floor(list.scrollTop / ROW) - OVERSCAN);
  const last = Math.min(audience.length, Math.ceil((list.scrollTop + height) / ROW) + OVERSCAN);
  const key = `${first}:${last}:${audience.length}`;
  if (!force && key === drawn) return;
  drawn = key;
  spacer.style.height = `${audience.length * ROW}px`;
  const rows = [];
  for (let i = first; i < last; i++) rows.push(row(audience[i], i));
  spacer.replaceChildren(...rows);
  list.setAttribute('aria-rowcount', String(audience.length));
  list.dataset.rendered = String(rows.length);
}
function build() {
  list = document.createElement('div');
  list.className = 'stage-audience-list';
  list.setAttribute('role', 'list');
  list.setAttribute('aria-label', 'Audience');
  list.tabIndex = 0;
  spacer = document.createElement('div');
  spacer.className = 'stage-audience-spacer';
  list.append(spacer);
  list.addEventListener('scroll', () => {
    top = list.scrollTop;
    if (frame) return;
    frame = requestAnimationFrame(() => { frame = 0; draw(false); });
  }, { passive: true });
}
/// The roster changed (from the Rust side).
export function roster(data) {
  audience = Array.isArray(data.audience) ? data.audience.slice(0, 1100) : [];
  me = String(data.me ?? '');
  moderator = data.moderator === true;
  draw(true);
}
/// After each render: the list lives on in its new host, where it was scrolled.
export function mount() {
  const host = document.querySelector('#stage-audience');
  if (!host) return;
  if (!list) build();
  if (list.parentElement !== host) {
    host.replaceChildren(list);
    list.scrollTop = top;
    draw(true);
  }
  const empty = host.querySelector('.stage-audience-empty');
  if (!audience.length && !empty) {
    const p = document.createElement('p');
    p.className = 'stage-audience-empty';
    p.textContent = 'No one is listening yet.';
    host.append(p);
  } else if (audience.length && empty) empty.remove();
}
export function stop() {
  if (frame) cancelAnimationFrame(frame);
  frame = 0;
  list?.remove();
  list = spacer = null;
  audience = [];
  top = 0;
  drawn = '';
}
export function diagnostics() {
  return { audience: audience.length, rendered: Number(list?.dataset.rendered ?? 0) };
}
