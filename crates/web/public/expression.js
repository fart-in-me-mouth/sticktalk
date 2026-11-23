// Press and hold a reaction chip (touch or pen) to use its other kind: a Super
// Reaction on a normal chip, a normal reaction on a Super one. Loaded on the
// first press; one timer exists only while a finger is down.
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
