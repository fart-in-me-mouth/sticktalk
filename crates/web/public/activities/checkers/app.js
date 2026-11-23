// Checkers: choose a piece, then a square. The server enforces captures,
// multi-jumps and kings; illegal moves are refused.
(async () => {
  const st = window.StickTalk;
  const { el } = window.StickTable;
  await st.ready();
  let picked = null;
  const table = window.StickTable.mount(document.getElementById('app'), {
    title: 'Checkers',
    sides: ['Dark', 'Light'],
    drawBoard(root, g, { mine, myTurn, act }) {
      const b = g.board;
      if (!b?.cells?.length) { root.replaceChildren(el('p', 'muted', 'The board appears when both seats are taken.')); return; }
      const owner = (p) => (p === 1 || p === 2 ? 0 : p === 3 || p === 4 ? 1 : -1);
      if (b.chain != null) picked = b.chain;
      // The second seat sees the board from its own side.
      const flip = mine === 1;
      const grid = el('div', 'ck');
      grid.setAttribute('role', 'grid');
      grid.setAttribute('aria-label', 'Board');
      for (let k = 0; k < 64; k++) {
        const i = flip ? 63 - k : k;
        const r = Math.floor(i / 8), c = i % 8;
        const dark = (r + c) % 2 === 1;
        const p = b.cells[i];
        const sq = el('button', `ck-sq ${dark ? 'dark' : 'light'}${picked === i ? ' picked' : ''}${b.last && (b.last[0] === i || b.last[1] === i) ? ' last' : ''}`);
        sq.type = 'button';
        sq.disabled = !dark || !myTurn;
        sq.setAttribute('aria-label', `${'abcdefgh'[c]}${8 - r}${p ? `, ${owner(p) === 0 ? 'dark' : 'light'} ${p % 2 === 0 ? 'king' : 'piece'}` : ''}`);
        if (p) sq.append(el('span', `ck-piece disc-${owner(p)}${p % 2 === 0 ? ' king' : ''}`, p % 2 === 0 ? '♛' : ''));
        sq.onclick = () => {
          if (owner(p) === mine && b.chain == null) { picked = picked === i ? null : i; table.render(); return; }
          if (picked != null && !p) { const from = picked; picked = null; act({ type: 'move', from, to: i }); }
        };
        grid.append(sq);
      }
      root.replaceChildren(grid);
      if (!myTurn) picked = null;
    },
  });
  document.body.dataset.ready = 'true';
})();
