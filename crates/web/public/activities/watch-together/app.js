// Watch Together: a shared queue of video and audio files stored on this
// StickTalk server, played in sync for everyone. The StickTalk page plays the
// file (through the call's output device and volume); this frame is the
// queue and the controls. The host controls playback, or everyone once the
// host opens control. State: `h/player` (the timeline), `q/<file>` (the
// queue, anyone may add), `s/media/<file>` (what the server shared).
(async () => {
  const st = window.StickTalk;
  const app = document.getElementById('app');
  const el = (tag, cls = '', text = '') => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  };
  await st.ready();
  let status = null;
  let advancing = null;
  let ui = {};

  const fmt = (s) => {
    if (!Number.isFinite(s)) return '–:––';
    s = Math.max(0, Math.floor(s));
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = String(s % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
  };
  const media = (id) => st.get(`s/media/${id}`);
  const queue = () => st.entries('q/')
    .map((e) => ({ id: e.key.slice(2), ...e.value, version: e.version }))
    .filter((q) => media(q.id))
    .sort((a, b) => (a.added ?? 0) - (b.added ?? 0));

  function play(id, position = 0) {
    return st.player.set({ item: id, playing: true, position, rate: st.player.get()?.rate ?? 1 }).catch(show);
  }
  function show(error) {
    const e = app.querySelector('.error');
    if (e) e.textContent = error?.message ?? '';
  }

  function render() {
    const ctx = st.context();
    const control = st.canControl();
    const p = st.player.get();
    const current = p?.item ? media(p.item) : null;
    app.replaceChildren();
    const bar = el('div', 'toolbar');
    const title = el('strong', 'now', current ? current.name : 'Nothing playing');
    const toggle = el('button', 'primary', p?.playing ? 'Pause' : 'Play');
    toggle.disabled = !control || !current;
    toggle.setAttribute('aria-label', p?.playing ? 'Pause' : 'Play');
    toggle.onclick = () => st.player.set({ item: p.item, playing: !p.playing, position: st.player.position(), rate: p.rate }).catch(show);
    const back = el('button', '', '−10 s');
    const fwd = el('button', '', '+10 s');
    back.setAttribute('aria-label', 'Back 10 seconds');
    fwd.setAttribute('aria-label', 'Forward 10 seconds');
    back.disabled = fwd.disabled = !control || !current;
    back.onclick = () => st.player.set({ ...p, position: Math.max(0, st.player.position() - 10) }).catch(show);
    fwd.onclick = () => st.player.set({ ...p, position: st.player.position() + 10 }).catch(show);
    const rate = el('select');
    rate.setAttribute('aria-label', 'Playback speed');
    for (const r of [0.5, 0.75, 1, 1.25, 1.5, 2]) rate.append(new Option(`${r}×`, String(r), false, (p?.rate ?? 1) === r));
    rate.disabled = !control || !current;
    rate.onchange = () => st.player.set({ ...p, position: st.player.position(), rate: Number(rate.value) }).catch(show);
    const time = el('span', 'muted time', status ? `${fmt(status.time)} / ${fmt(status.duration)}` : '');
    bar.append(toggle, back, fwd, rate, title, time);
    const seek = el('input', 'seek');
    seek.type = 'range';
    seek.min = '0';
    seek.max = String(Math.max(1, Math.round(status?.duration ?? 0)));
    seek.value = String(Math.round(status?.time ?? 0));
    seek.disabled = !control || !current || !status?.duration;
    seek.setAttribute('aria-label', 'Position');
    seek.onchange = () => st.player.set({ ...p, position: Number(seek.value) }).catch(show);
    const who = el('p', 'status', control
      ? (ctx.shared ? 'Everyone can control playback.' : 'You control playback.')
      : `${ctx.participants.find((x) => x.pid === ctx.host)?.name ?? 'The host'} controls playback.`);
    if (status?.problem === 'blocked') who.textContent = 'Your browser blocked playback. Press Play in the tile to start.';
    if (status?.buffering) who.textContent += ' Loading…';
    const list = el('ol', 'queue');
    list.setAttribute('aria-label', 'Queue');
    for (const q of queue()) {
      const li = el('li', q.id === p?.item ? 'playing' : '');
      li.append(el('span', 'name', `${q.id === p?.item ? '▶ ' : ''}${media(q.id).name}`), el('small', 'muted', st.participant(q.by)?.name ?? ''));
      const go = el('button', '', 'Play');
      go.disabled = !control;
      go.setAttribute('aria-label', `Play ${media(q.id).name}`);
      go.onclick = () => play(q.id);
      const remove = el('button', '', 'Remove');
      remove.disabled = !(control || q.by === ctx.you);
      remove.setAttribute('aria-label', `Remove ${media(q.id).name}`);
      remove.onclick = () => st.remove(`q/${q.id}`).catch(show);
      li.append(go, remove);
      list.append(li);
    }
    const add = el('button', 'primary', 'Add a Video or Audio File');
    add.onclick = async () => {
      try {
        const item = await st.shareMedia();
        await st.set(`q/${item.id}`, { by: st.context().you, added: st.serverNow() });
        if (!st.player.get()?.item && st.canControl()) play(item.id);
      } catch (e) {
        if (e.message !== 'cancelled') show(e);
      }
    };
    const empty = queue().length ? null : el('p', 'status', 'The queue is empty. Add a file from this server to watch or listen together.');
    const actions = el('div', 'row actions');
    actions.append(add);
    if (st.isHost()) actions.append(el('span', 'muted', 'Open control to everyone from the tile’s options.'));
    app.append(bar, seek, who, ...(empty ? [empty] : []), list, actions, el('p', 'error'));
    ui = { time, seek };
  }
  /** Only the clock and the position bar, while playing. */
  function tick() {
    if (!ui.time) return;
    ui.time.textContent = status ? `${fmt(status.time)} / ${fmt(status.duration)}` : '';
    if (document.activeElement !== ui.seek) {
      ui.seek.max = String(Math.max(1, Math.round(status?.duration ?? 0)));
      ui.seek.value = String(Math.round(status?.time ?? 0));
      ui.seek.disabled = !st.canControl() || !status?.duration;
    }
  }

  // The host moves to the next file when one ends.
  st.on('player', (s) => {
    const was = status;
    status = s;
    const p = st.player.get();
    if (s?.ended && p?.playing && st.isHost() && advancing !== p.item) {
      advancing = p.item;
      const q = queue();
      const i = q.findIndex((x) => x.id === p.item);
      const next = q[i + 1];
      st.player.set(next ? { item: next.id, playing: true, position: 0, rate: p.rate } : { ...p, playing: false, position: s.duration ?? p.position }).catch(() => {});
    }
    if (!was || !s || was.item !== s.item || was.paused !== s.paused || was.ended !== s.ended || was.buffering !== s.buffering || was.problem !== s.problem || !was.duration !== !s.duration) render();
    else tick();
  });
  st.on('state', () => render());
  st.on('participants', () => render());
  st.resize(200).catch(() => {});
  render();
  document.body.dataset.ready = 'true';
})();
