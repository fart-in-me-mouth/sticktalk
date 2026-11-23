// Four in a Row: the server checks every move and finds the winner.
(async () => {
  const st = window.StickTalk;
  const { el } = window.StickTable;
  await st.ready();
  window.StickTable.mount(document.getElementById('app'), {
    title: 'Four in a Row',
    sides: ['Red', 'Yellow'],
    drawBoard(root, g, { myTurn, act }) {
      const b = g.board;
      if (!b?.cells?.length) { root.replaceChildren(el('p', 'muted', 'The board appears when both seats are taken.')); return; }
      const grid = el('div', 'c4');
      grid.setAttribute('role', 'grid');
      grid.setAttribute('aria-label', 'Board');
      grid.style.setProperty('--cols', b.width);
      for (let c = 0; c < b.width; c++) {
        const col = el('button', 'c4-col');
        col.type = 'button';
        col.disabled = !myTurn || b.cells[c] !== 0;
        col.setAttribute('aria-label', `Drop in column ${c + 1}`);
        col.onclick = () => act({ type: 'move', col: c });
        for (let r = 0; r < b.height; r++) {
          const i = r * b.width + c;
          const cell = el('span', `c4-cell${b.cells[i] ? ` disc-${b.cells[i] - 1}` : ''}${b.last === i ? ' last' : ''}`);
          col.append(cell);
        }
        grid.append(col);
      }
      root.replaceChildren(grid);
    },
  });
  document.body.dataset.ready = 'true';
})();
