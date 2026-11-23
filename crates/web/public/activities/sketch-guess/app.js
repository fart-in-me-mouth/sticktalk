// Sketch & Guess: the server picks a word and tells only the person drawing.
// Their strokes are keys `d/<n>` (only they may write them this round); the
// others guess, and the server decides who is right, keeps the scores and
// screens every guess like a message.
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
  /* theme-lint: artwork */
  const INK = ['#212126', '#ff5d5d', '#3b82f6', '#22c55e', '#f59e0b', '#a855f7'];
  const PAPER = '#fbfbf7';
  /* theme-lint: end */
  const W = 800, H = 600;
  let word = '';
  let close = false;
  let ink = 0;
  let n = 0;
  let error = '';

  const head = el('div', 'toolbar');
  const main = el('div', 'sketch-main');
  const paper = el('div', 'paper');
  const canvas = el('canvas');
  canvas.width = W;
  canvas.height = H;
  canvas.setAttribute('aria-label', 'Drawing');
  paper.append(canvas);
  const side = el('aside', 'side');
  const scores = el('ol', 'scores');
  scores.setAttribute('aria-label', 'Scores');
  const chat = el('ul', 'chat');
  chat.setAttribute('aria-label', 'Guesses');
  chat.setAttribute('aria-live', 'polite');
  // Not a <form>: the sandbox does not submit forms.
  const form = el('div', 'guess');
  const input = el('input');
  input.type = 'text';
  input.maxLength = 60;
  input.placeholder = 'Type your guess';
  input.setAttribute('aria-label', 'Your guess');
  const send = el('button', 'primary', 'Guess');
  send.type = 'button';
  form.append(input, send);
  side.append(scores, chat, form);
  main.append(paper, side);
  app.replaceChildren(head, main);

  const act = (data) => st.action(data).then(() => { error = ''; render(); }, (e) => { error = e.message; render(); });
  const guess = () => {
    const text = input.value.trim();
    if (!text) return;
    input.value = '';
    close = false;
    act({ type: 'guess', text });
  };
  send.onclick = guess;
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); guess(); }
  });

  const game = () => st.get('s/game') ?? { status: 'lobby', scores: [], chat: [], guessed: [] };
  const drawing = () => { const g = game(); return g.status === 'drawing' && g.drawer === st.context().you; };

  function render() {
    const g = game();
    const ctx = st.context();
    head.replaceChildren();
    const info = el('strong', 'info');
    if (g.status === 'lobby' || g.status === 'over') {
      info.textContent = g.status === 'over' ? 'Game over!' : 'Sketch & Guess';
      const start = el('button', 'primary', g.status === 'over' ? 'Play Again' : 'Start Game');
      start.disabled = ctx.participants.length < 2;
      start.title = start.disabled ? 'Needs at least two people' : '';
      start.onclick = () => act({ type: 'start' });
      head.append(info, start);
      if (start.disabled) head.append(el('span', 'muted', 'Waiting for at least two people.'));
    } else if (g.status === 'drawing') {
      const who = st.participant(g.drawer)?.name ?? 'Someone';
      info.textContent = drawing() ? `Draw: ${word || '…'}` : `${who} is drawing`;
      head.append(info, el('span', 'hint', drawing() ? '' : g.hint.split('').join(' ')), el('span', 'muted', `Round ${g.round} of ${g.rounds}`));
      if (drawing()) {
        INK.forEach((c, i) => {
          const b = el('button', 'ink');
          b.type = 'button';
          b.style.background = c;
          b.setAttribute('aria-label', `Ink ${i + 1}`);
          b.setAttribute('aria-pressed', String(i === ink));
          b.onclick = () => { ink = i; render(); };
          head.append(b);
        });
        const clear = el('button', '', 'Clear');
        clear.onclick = () => {
          const keys = st.entries('d/').map((e) => e.key);
          for (let i = 0; i < keys.length; i += 32) st.batch(keys.slice(i, i + 32).map((key) => ({ key, value: null }))).catch(() => {});
        };
        const skip = el('button', '', 'Skip');
        skip.onclick = () => act({ type: 'skip' });
        head.append(clear, skip);
      }
    } else if (g.status === 'reveal') {
      info.textContent = `The word was “${g.word}”`;
      const next = el('button', 'primary', g.round >= g.rounds ? 'Finish' : 'Next Round');
      next.onclick = () => act({ type: 'next' });
      head.append(info, next);
    }
    if (error) head.append(el('span', 'error', error));
    if (close) head.append(el('span', 'close', 'So close!'));
    scores.replaceChildren(...[...g.scores].sort((a, b) => b.score - a.score).map((s) => {
      const li = el('li', g.guessed.includes(s.pid) ? 'got' : '');
      li.append(el('span', '', `${s.name}${s.pid === g.drawer ? ' ✎' : ''}`), el('strong', '', String(s.score)));
      return li;
    }));
    chat.replaceChildren(...g.chat.map((m) => el('li', m.ok ? 'ok' : '', m.who && !m.ok ? `${m.who}: ${m.text}` : m.text)));
    chat.scrollTop = chat.scrollHeight;
    const guessing = g.status === 'drawing' && !drawing() && !g.guessed.includes(ctx.you);
    input.disabled = send.disabled = !guessing;
    canvas.style.cursor = drawing() ? 'crosshair' : 'default';
    paint();
  }

  // ---- the drawing -------------------------------------------------------------------
  const live = new Map();
  let stroke = null;
  function line(g, s) {
    if (!s.pts?.length) return;
    g.strokeStyle = INK[s.c] ?? INK[0];
    g.lineWidth = s.w ?? 6;
    g.lineCap = g.lineJoin = 'round';
    g.beginPath();
    s.pts.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)));
    if (s.pts.length === 1) g.lineTo(s.pts[0][0] + 0.1, s.pts[0][1]);
    g.stroke();
  }
  let frame = 0;
  function paint() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      const g = canvas.getContext('2d');
      g.fillStyle = PAPER;
      g.fillRect(0, 0, W, H);
      for (const e of st.entries('d/').sort((a, b) => a.version - b.version)) line(g, e.value);
      for (const s of live.values()) line(g, s);
      if (stroke) line(g, stroke);
    });
  }
  const point = (e) => {
    const r = canvas.getBoundingClientRect();
    return [Math.round((e.clientX - r.left) * W / r.width), Math.round((e.clientY - r.top) * H / r.height)];
  };
  let timer = 0, sent = 0;
  function flush() {
    timer = 0;
    if (!stroke) return;
    const from = Math.max(0, sent - 1);
    const pts = stroke.pts.slice(from, from + 100);
    sent = from + pts.length;
    st.send({ live: { c: stroke.c, from, pts } }).catch(() => {});
    if (sent < stroke.pts.length) timer = setTimeout(flush, 80);
  }
  canvas.addEventListener('pointerdown', (e) => {
    if (!drawing()) return;
    canvas.setPointerCapture(e.pointerId);
    stroke = { c: ink, w: 6, pts: [point(e)] };
    sent = 0;
    paint();
  });
  canvas.addEventListener('pointermove', (e) => {
    if (!stroke) return;
    const p = point(e), last = stroke.pts.at(-1);
    if (Math.hypot(p[0] - last[0], p[1] - last[1]) < 2 || stroke.pts.length >= 800) return;
    stroke.pts.push(p);
    if (!timer) timer = setTimeout(flush, 80);
    paint();
  });
  const end = () => {
    if (!stroke) return;
    const s = stroke;
    stroke = null;
    clearTimeout(timer);
    timer = 0;
    st.set(`d/${st.context().you}-${++n}`, s).then(() => st.send({ done: true }), (e) => { error = e.message; render(); });
    paint();
  };
  canvas.addEventListener('pointerup', end);
  canvas.addEventListener('pointercancel', end);

  st.on('message', ({ from, data }) => {
    if (from === 0 && data?.type === 'word') { word = data.word; render(); return; }
    if (from === 0 && data?.type === 'close') { close = true; render(); return; }
    if (data?.live && Array.isArray(data.live.pts)) {
      const pts = data.live.pts.filter((p) => Array.isArray(p) && p.length === 2).slice(0, 200);
      const s = live.get(from);
      if (!s || !data.live.from) live.set(from, { c: data.live.c, w: 6, pts });
      else if (s.pts.length < 1600) s.pts.push(...pts.slice(1));
      paint();
    }
    if (data?.done) { live.delete(from); paint(); }
  });
  st.on('state', ({ entries }) => {
    if (entries.some((e) => e.key === 's/game')) {
      const g = game();
      if (g.status !== 'drawing') word = g.status === 'reveal' || g.status === 'over' ? g.word : '';
      if (g.status !== 'drawing') live.clear();
      render();
    } else paint();
  });
  st.on('participants', render);
  render();
  document.body.dataset.ready = 'true';
})();
