// UI SDK (https://github.com/vercel-labs/ui) behaviors for the Rust views:
// dialogs (top layer, focus trap, dismissal), tooltips, dropdown menus and
// avatar image fallbacks. The elements are unstyled; styles/app.css styles them.
// The packager copies the SDK's browser modules next to this file.
import '/public/ui-sdk/web/avatar.js';

// Dialogs, menus and tooltips are defined when first needed (docs/features/perf.md):
// most of the SDK's code is theirs, and the first screen does not use them until
// a person points at, focuses or presses one. Until then their content stays
// hidden (it is rendered with `hidden`), and the triggers are ordinary buttons
// with their own labels.
const modules = {
  'ui-dialog': () => import('/public/ui-sdk/web/dialog.js'),
  'ui-menu': () => import('/public/ui-sdk/web/menu.js'),
  'ui-tooltip': () => import('/public/ui-sdk/web/tooltip.js'),
};
const loading = new Map();
export function define(tag) {
  let done = loading.get(tag);
  if (!done) {
    done = modules[tag]().then(() => customElements.whenDefined(tag));
    loading.set(tag, done);
    done.catch(() => loading.delete(tag));
  }
  return done;
}
// A dialog is opened by the app: define it as soon as one is in the page.
const dialogs = document.getElementsByTagName('ui-dialog');
const watch = new MutationObserver(() => {
  if (!dialogs.length) return;
  watch.disconnect();
  void define('ui-dialog');
});
watch.observe(document.documentElement, { childList: true, subtree: true });
// Intent. Tooltips: the first pointer movement or focus anywhere (a person is
// using the page, not just looking); by the time the pointer rests on a
// button they are defined. Menus and dialogs: pointing at or focusing one, or
// anything that opens a dialog, downloads its behavior before it is pressed.
const intent = (event) => {
  if (!customElements.get('ui-tooltip')) void define('ui-tooltip');
  const target = event.target;
  if (!(target instanceof Element)) return;
  if (!customElements.get('ui-menu') && target.closest('ui-menu')) void define('ui-menu');
  if (!customElements.get('ui-dialog') && target.closest('[data-live^="dialog:"],ui-menu')) void define('ui-dialog');
};
document.addEventListener('pointerover', intent, { capture: true, passive: true });
document.addEventListener('focusin', intent, { capture: true });
// Pressed before its behavior arrived (a fast tap): hold the press, then open it.
const press = (event) => {
  const target = event.target;
  if (!(target instanceof Element) || customElements.get('ui-menu')) return;
  const trigger = target.closest('ui-menu ui-menu-trigger');
  if (!trigger) return;
  if (event.type === 'keydown' && !['Enter', ' ', 'ArrowDown', 'ArrowUp'].includes(event.key)) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  if (event.type === 'pointerdown') return;
  void define('ui-menu').then(() => (trigger.querySelector('button') ?? trigger).click());
};
for (const type of ['pointerdown', 'click', 'keydown']) document.addEventListener(type, press, { capture: true });

// Sent as a request: live.js asks for confirmation where an action needs one.
const emit = (action) =>
  window.dispatchEvent(new CustomEvent('sticktalk-request', { detail: action }));

// SDK change events do not bubble; listen in the capture phase.
// The app state owns which dialog is open, so a dismissal is a request to it,
// never a local state change. Only a person's own dismissal counts: the SDK also
// reports the dialog the app just removed, by which time another may be open.
// Escape is already an app-wide shortcut in live.js.
const dismissals = new Set(['outside-press', 'back', 'swipe', 'close-press']);
document.addEventListener('openchange', (event) => {
  const dialog = event.target;
  if (dialog.localName !== 'ui-dialog' || event.detail.value !== false) return;
  event.preventDefault();
  // A press inside the image viewer, the message menu or a confirmation, which
  // open above a dialog, is not a press outside that dialog.
  if (!dialog.isConnected || !dismissals.has(event.detail.reason) || document.querySelector('#live-viewer,#live-message-menu,#live-confirm,dialog[open]')) return;
  emit('close');
}, { capture: true });

// A menu item's value is an app action, exactly like a data-live attribute.
document.addEventListener('action', (event) => {
  const value = event.detail?.value;
  if (event.target.localName === 'ui-menu' && typeof value === 'string' && value) emit(value);
}, { capture: true });
