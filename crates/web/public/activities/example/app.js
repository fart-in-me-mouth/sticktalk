// A minimal activity (docs/ACTIVITIES.md): a counter everyone shares, and an
// ephemeral wave. It uses only the SDK, as a registered activity would.
(async () => {
  const st = window.StickTalk;
  const $ = (id) => document.getElementById(id);
  const ctx = await st.ready();
  const show = () => {
    $('count').textContent = String(st.get('count') ?? 0);
    const c = st.context();
    $('people').textContent = c.participants.map((p) => p.name + (p.pid === c.host ? ' (host)' : '')).join(', ');
  };
  st.on('state', show);
  st.on('participants', show);
  st.on('message', ({ from, data }) => {
    if (data?.wave) $('log').textContent = `${st.participant(from)?.name ?? 'Someone'} waved.`;
  });
  $('add').onclick = async () => {
    // Compare-and-set: two people pressing at once both count.
    for (let i = 0; i < 3; i++) {
      try {
        await st.set('count', (st.get('count') ?? 0) + 1, { expect: st.versionOf('count') });
        return;
      } catch { /* someone else was first; try again with their value */ }
    }
  };
  $('wave').onclick = () => st.send({ wave: true });
  show();
  document.body.dataset.ready = String(ctx.you);
})();
