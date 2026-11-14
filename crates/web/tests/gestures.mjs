import { engine } from './engines.mjs';
import { setTimeout as wait } from 'node:timers/promises';

// Chromium exposes native multi-touch through CDP. Playwright's public WebKit /
// Firefox API only exposes tap: dispatch DOM pointer/touch sequences there.
// These test handler behavior, NOT OS gesture recognition or iOS callout policy.
export async function gesture(page, frames) {
  if (engine === 'chromium') {
    const cdp = await page.context().newCDPSession(page);
    try {
      for (const { type, points = [], delay = 0 } of frames) {
        await cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points });
        if (delay) await wait(delay);
      }
    } finally { await cdp.detach(); }
    return;
  }
  for (const { type, points = [], delay = 0 } of frames) {
    await page.evaluate(({ type, points }) => {
      const active = window.__qaPointers ??= new Map();
      const eventType = { touchStart: 'pointerdown', touchMove: 'pointermove', touchEnd: 'pointerup' }[type];
      const changed = type === 'touchEnd' ? [...active.values()] : points;
      for (const point of changed) {
        const target = active.get(point.id)?.target ?? document.elementFromPoint(point.x, point.y);
        if (!target) throw Error('No gesture target');
        // DOM-dispatched IDs are not OS-active and cannot request native capture.
        // Override only this element during dispatch, then restore immediately.
        const own = Object.getOwnPropertyDescriptor(target, 'setPointerCapture');
        Object.defineProperty(target, 'setPointerCapture', { configurable: true, value() {} });
        try {
          target.dispatchEvent(new PointerEvent(eventType, { bubbles: true, cancelable: true,
            pointerType: 'touch', pointerId: point.id, clientX: point.x, clientY: point.y,
            buttons: type === 'touchEnd' ? 0 : 1, isPrimary: point.id === 1 }));
        } finally {
          if (own) Object.defineProperty(target, 'setPointerCapture', own); else delete target.setPointerCapture;
        }
        if (type === 'touchEnd') active.delete(point.id);
        else active.set(point.id, { ...point, target });
      }
      const target = changed[0] && (active.get(changed[0].id)?.target ?? document.elementFromPoint(changed[0].x, changed[0].y));
      if (target && typeof Touch === 'function') {
        const touch = p => new Touch({ identifier: p.id, target, clientX: p.x, clientY: p.y });
        target.dispatchEvent(new TouchEvent(type.toLowerCase(), { bubbles: true, cancelable: true,
          touches: [...active.values()].map(touch), changedTouches: changed.map(touch) }));
      }
    }, { type, points });
    if (delay) await wait(delay);
  }
}
export const swipe = (page, from, to) => gesture(page, [
  { type: 'touchStart', points: [{ ...from, id: 1 }] },
  { type: 'touchMove', points: [{ ...to, id: 1 }] },
  { type: 'touchEnd' },
]);
