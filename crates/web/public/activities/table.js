// The table around a two-player board game the server runs (four-in-a-row,
// checkers): two seats, whose turn it is, resign, rematch, and everyone
// else watching. The game itself is `s/game`, written only by the server.
window.StickTable = (() => {
  'use strict';
  const el = (tag, cls = '', text = '') => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  };
  function mount(root, { title, sides, drawBoard }) {
    const st = window.StickTalk;
    const head = el('div', 'table-head');
    const board = el('div', 'table-board');
    const foot = el('p', 'status');
    foot.setAttribute('role', 'status');
    foot.setAttribute('aria-live', 'polite');
    root.replaceChildren(head, board, foot);
    let error = '';
    const act = (data) => st.action(data).then(() => { error = ''; render(); }, (e) => { error = e.message; render(); });
    function render() {
      const g = st.get('s/game');
      const ctx = st.context();
      if (!g || !ctx) return;
      const mine = g.seats.findIndex((s) => s && s.pid === ctx.you);
      head.replaceChildren();
      g.seats.forEach((s, i) => {
        const seat = el('div', `seat seat-${i}${g.status === 'playing' && g.turn === i ? ' turn' : ''}`);
        seat.append(el('span', `disc disc-${i}`), el('strong', '', s ? s.name : 'Open seat'), el('small', 'muted', sides[i]));
        if (!s && mine < 0 && g.status !== 'playing') {
          const sit = el('button', 'primary', 'Sit Here');
          sit.setAttribute('aria-label', `Play ${sides[i]}`);
          sit.onclick = () => act({ type: 'sit', seat: i });
          seat.append(sit);
        }
        head.append(seat);
      });
      const tools = el('div', 'row table-tools');
      if (mine >= 0 && g.status === 'playing') {
        const resign = el('button', '', 'Resign');
        resign.onclick = () => act({ type: 'resign' });
        tools.append(resign);
      }
      if (mine >= 0 && g.status !== 'playing') {
        const stand = el('button', '', 'Leave Seat');
        stand.onclick = () => act({ type: 'stand' });
        tools.append(stand);
      }
      if (mine >= 0 && g.status === 'over' && g.seats.every(Boolean)) {
        const again = el('button', 'primary', g.rematch[mine] ? 'Waiting for Rematch…' : 'Rematch');
        again.disabled = g.rematch[mine];
        again.onclick = () => act({ type: 'rematch' });
        tools.append(again);
      }
      head.append(tools);
      drawBoard(board, g, { mine, myTurn: g.status === 'playing' && g.turn === mine, act });
      const name = (i) => g.seats[i]?.name ?? sides[i];
      let text;
      if (g.status === 'waiting') text = g.seats.filter(Boolean).length === 1 ? 'Waiting for a second player.' : `Take a seat to play ${title}.`;
      else if (g.status === 'playing') text = g.turn === mine ? 'Your turn.' : `${name(g.turn)}’s turn.`;
      else text = g.winner == null ? 'A draw.' : `${name(g.winner)} wins!`;
      if (mine < 0 && g.status === 'playing') text += ' You are watching.';
      foot.textContent = error ? `${text} ${error}` : text;
      foot.classList.toggle('error', !!error);
    }
    st.on('state', render);
    st.on('participants', render);
    render();
    return { render };
  }
  return { mount, el };
})();
