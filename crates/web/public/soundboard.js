// The soundboard panel (loaded when it is first opened in a call) and the
// entrance-sound settings. Sounds of the call's own server come first, then
// every other server you are in; favorites sit on top. Managing sounds (add,
// rename, change emoji or volume, delete) happens here too, for people with
// Manage Soundboard or Manage Server in that server.
const $ = (s) => document.querySelector(s);
let tools = null, host = null;
const SVG = 'http://www.w3.org/2000/svg';
function glyph(name) {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.classList.add('icon');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(SVG, 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}
function iconButton(name, label, onclick, className = 'icon-button') {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = className;
  b.setAttribute('aria-label', label);
  b.title = label;
  b.append(glyph(name));
  b.onclick = onclick;
  return b;
}
const element = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};

/** Sounds matching a search, favorites first, then each server in order. */
export function sections(library, query = '') {
  const q = query.trim().toLowerCase();
  const match = (s) => !q || s.name.toLowerCase().includes(q) || (s.emoji ?? '').includes(q);
  const favorites = new Set(library.favorites ?? []);
  const all = library.guilds.flatMap((g) => g.sounds);
  const out = [];
  const fav = all.filter((s) => favorites.has(s.id) && match(s));
  if (fav.length) out.push({ title: 'Favorites', sounds: fav, guild: null });
  for (const g of library.guilds) {
    const sounds = g.sounds.filter(match);
    if (sounds.length || (g.can_manage && !q))
      out.push({
        title: g.guild_id === library.current_guild_id ? `${g.name} · This Server` : g.name,
        sounds,
        guild: g,
      });
  }
  return out;
}

let panel = null;
export async function open(callHost, t) {
  host = callHost;
  tools = t;
  if (panel?.isConnected) {
    panel.querySelector('input[type="search"]')?.focus();
    return;
  }
  const d = tools.dialog('Soundboard', { className: 'soundboard-panel' });
  panel = d;
  const status = element('p', { className: 'muted-copy soundboard-status' });
  status.setAttribute('role', 'status');
  const search = element('input', { type: 'search', placeholder: 'Search sounds', className: 'soundboard-search' });
  search.setAttribute('aria-label', 'Search sounds');
  const list = element('div', { className: 'soundboard-list' });
  const settings = element('details', { className: 'soundboard-settings' }, element('summary', { textContent: 'Soundboard volume' }), tools.soundControls());
  const close = tools.button('Close', 'button secondary', () => d.close());
  d.append(search, status, list, settings, close);
  d.showModal();
  search.focus();
  let library = null;
  const render = () => {
    if (!library) return;
    list.replaceChildren();
    const groups = sections(library, search.value);
    if (!library.can_play)
      status.textContent = 'You don’t have permission to use the soundboard in this channel.';
    else if (!groups.some((g) => g.sounds.length))
      status.textContent = search.value ? 'No sounds match.' : 'No sounds yet. Servers you manage can add up to 48 each.';
    else status.textContent = '';
    for (const group of groups) {
      const section = element('section', { className: 'soundboard-group' });
      const header = element('div', { className: 'soundboard-group-header' }, element('h3', { className: 'category', textContent: group.title }));
      if (group.guild?.can_manage)
        header.append(tools.button('Add Sound', 'button secondary soundboard-add', () => editor(group.guild, null, reload)));
      const grid = element('div', { className: 'soundboard-grid' });
      grid.setAttribute('role', 'list');
      for (const sound of group.sounds) grid.append(tile(sound, group, library, reload));
      section.append(header, grid);
      list.append(section);
    }
  };
  const reload = async () => {
    try {
      library = await tools.api(`/api/v1/soundboard?channel_id=${encodeURIComponent(host.channel)}`);
      render();
    } catch (e) {
      status.textContent = e.message;
    }
  };
  search.oninput = render;
  status.textContent = 'Loading sounds…';
  await reload();
}
function tile(sound, group, library, reload) {
  const item = element('div', { className: 'soundboard-item' });
  item.setAttribute('role', 'listitem');
  const playButton = element('button', { type: 'button', className: 'soundboard-sound' },
    element('span', { className: 'soundboard-emoji', textContent: sound.emoji || '🔊', ariaHidden: 'true' }),
    element('span', { className: 'soundboard-name', textContent: sound.name }));
  playButton.setAttribute('aria-label', `Play ${sound.name}`);
  playButton.disabled = !library.can_play;
  playButton.dataset.sound = sound.id;
  playButton.onclick = () => {
    // Everyone hears it from the server's event, you included; nothing local here.
    host.signal('play_sound', { sound_id: sound.id });
    playButton.classList.add('is-played');
    setTimeout(() => playButton.classList.remove('is-played'), 400);
  };
  const favorites = new Set(library.favorites ?? []);
  const starred = favorites.has(sound.id);
  const star = iconButton('star', starred ? `Remove ${sound.name} from favorites` : `Add ${sound.name} to favorites`, async () => {
    try {
      await tools.api(`/api/v1/soundboard/favorites/${sound.id}`, { method: starred ? 'DELETE' : 'PUT' });
      await reload();
    } catch (e) { host.report(e); }
  }, `icon-button soundboard-star${starred ? ' is-favorite' : ''}`);
  star.setAttribute('aria-pressed', String(starred));
  item.append(playButton, star);
  const guild = library.guilds.find((g) => g.guild_id === sound.guild_id);
  if (guild?.can_manage && group.guild)
    item.append(iconButton('pencil', `Edit ${sound.name}`, () => editor(guild, sound, reload), 'icon-button soundboard-edit'));
  return item;
}
/** Add a sound to a server, or change or delete one. */
function editor(guild, sound, reload) {
  const d = tools.dialog(sound ? `Edit ${sound.name}` : `Add a Sound to ${guild.name}`, { className: 'soundboard-editor' });
  const form = element('form', { className: 'soundboard-form', noValidate: true });
  const field = (label, control, help) => {
    const f = element('label', { className: 'field' }, label, control);
    if (help) f.append(element('small', { textContent: help }));
    return f;
  };
  const file = element('input', { type: 'file', accept: '.mp3,.ogg,.oga,.opus,.wav,audio/mpeg,audio/ogg,audio/wav' });
  const name = element('input', { type: 'text', maxLength: 32, required: true, value: sound?.name ?? '' });
  const emoji = element('input', { type: 'text', maxLength: 16, value: sound?.emoji ?? '', placeholder: '🔊' });
  const volume = element('input', { type: 'range', min: '0', max: '100', value: String(sound?.volume ?? 100) });
  volume.setAttribute('aria-valuetext', `${volume.value}%`);
  volume.oninput = () => volume.setAttribute('aria-valuetext', `${volume.value}%`);
  const status = element('p', { className: 'muted-copy' });
  status.setAttribute('role', 'status');
  if (!sound) {
    form.append(field('Sound File', file, 'MP3, OGG or WAV, at most 5.2 seconds and 512 KB.'));
    file.onchange = () => {
      const f = file.files[0];
      if (f && !name.value) name.value = f.name.replace(/\.[^.]+$/, '').slice(0, 32);
      if (f && f.size > 512 * 1024) status.textContent = 'That file is larger than 512 KB.';
      else status.textContent = '';
    };
  }
  form.append(field('Name', name), field('Emoji', emoji, 'Optional: one emoji shown on the button.'), field('Volume', volume, 'How loud it plays for everyone, before their own soundboard volume.'), status);
  const save = tools.button(sound ? 'Save' : 'Upload', 'button primary', null);
  save.type = 'submit';
  const actions = element('div', { className: 'soundboard-form-actions' }, save, tools.button('Cancel', 'button secondary', () => d.close()));
  if (sound)
    actions.append(tools.button('Delete Sound', 'button danger-text', async () => {
      if (!confirmDelete(sound)) return;
      try {
        await tools.api(`/api/v1/soundboard/sounds/${sound.id}`, { method: 'DELETE' });
        d.close();
        await reload();
      } catch (e) { status.textContent = e.message; }
    }));
  form.append(actions);
  form.onsubmit = async (e) => {
    e.preventDefault();
    const trimmed = name.value.trim();
    if (!trimmed) { status.textContent = 'Give the sound a name.'; name.focus(); return; }
    save.disabled = true;
    status.textContent = sound ? 'Saving…' : 'Uploading…';
    try {
      if (sound) {
        await tools.api(`/api/v1/soundboard/sounds/${sound.id}`, { method: 'PATCH', json: { name: trimmed, emoji: emoji.value.trim(), volume: Number(volume.value) } });
      } else {
        const f = file.files[0];
        if (!f) throw new Error('Choose a sound file.');
        const query = new URLSearchParams({ name: trimmed, volume: volume.value });
        if (emoji.value.trim()) query.set('emoji', emoji.value.trim());
        await tools.api(`/api/v1/guilds/${guild.guild_id}/soundboard?${query}`, { method: 'POST', body: f });
      }
      d.close();
      await reload();
    } catch (err) {
      status.textContent = err.message;
      save.disabled = false;
    }
  };
  d.append(form);
  d.showModal();
  (sound ? name : file).focus();
}
function confirmDelete(sound) {
  // A plain confirmation keeps a mis-tap from deleting a server's sound.
  return globalThis.__sticktalkConfirm ? globalThis.__sticktalkConfirm(sound) : confirm(`Delete ${sound.name} for everyone in this server?`);
}

// ---- entrance sounds (Voice & Video settings) ----------------------------------
export async function entranceSettings(root) {
  const t = await import('./callextras.js');
  let library, current;
  try {
    [library, current] = await Promise.all([t.api('/api/v1/soundboard'), t.api('/api/v1/soundboard/entrance')]);
  } catch (e) {
    root.textContent = e.message;
    return;
  }
  const status = element('p', { className: 'muted-copy' });
  status.setAttribute('role', 'status');
  const options = (select, value, extra) => {
    for (const [v, text] of extra) select.add(new Option(text, v));
    for (const g of library.guilds) {
      if (!g.sounds.length) continue;
      const group = element('optgroup', { label: g.name });
      for (const s of g.sounds) group.append(new Option(`${s.emoji ? `${s.emoji} ` : ''}${s.name}`, s.id));
      select.append(group);
    }
    select.value = value;
  };
  const enabled = element('input', { type: 'checkbox', checked: current.enabled });
  const toggle = element('label', { className: 'live-checkbox' }, element('span', { textContent: 'Play My Entrance Sound' }), enabled);
  const fallback = element('select');
  fallback.setAttribute('aria-label', 'Entrance sound');
  options(fallback, current.default_sound_id ?? '', [['', 'None']]);
  const overrides = element('div', { className: 'entrance-overrides' });
  const rows = new Map();
  for (const g of library.guilds) {
    const select = element('select');
    select.setAttribute('aria-label', `Entrance sound in ${g.name}`);
    const o = current.overrides.find((x) => x.guild_id === g.guild_id);
    options(select, o ? (o.sound_id ?? 'none') : '', [['', 'Same as default'], ['none', 'None in this server']]);
    rows.set(g.guild_id, select);
    overrides.append(element('label', { className: 'field' }, g.name, select));
  }
  const details = element('details', { className: 'entrance-per-server' }, element('summary', { textContent: 'Different sound in a server' }), overrides);
  const save = async () => {
    const body = {
      enabled: enabled.checked,
      default_sound_id: fallback.value || null,
      overrides: [...rows].filter(([, s]) => s.value).map(([guild_id, s]) => ({ guild_id, sound_id: s.value === 'none' ? null : s.value })),
    };
    try {
      await t.api('/api/v1/soundboard/entrance', { method: 'PUT', json: body });
      status.textContent = 'Saved.';
    } catch (e) {
      status.textContent = e.message;
    }
  };
  for (const c of [enabled, fallback, ...rows.values()]) c.addEventListener('change', save);
  const help = element('p', { className: 'muted-copy', textContent: 'Plays for everyone in a voice channel when you join it, at most once a minute. Others can turn off hearing entrance sounds.' });
  root.replaceChildren(toggle, element('label', { className: 'field' }, 'Default Sound', fallback), library.guilds.length ? details : '', help, status);
}
