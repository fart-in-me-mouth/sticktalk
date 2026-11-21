// UI SDK (https://github.com/vercel-labs/ui) behaviors for the Rust views:
// dialogs (top layer, focus trap, dismissal), tooltips, dropdown menus and
// avatar image fallbacks. The elements are unstyled; styles/app.css styles them.
// The packager copies the SDK's browser modules next to this file.
import '/public/ui-sdk/web/avatar.js';
import '/public/ui-sdk/web/dialog.js';
import '/public/ui-sdk/web/menu.js';
import '/public/ui-sdk/web/tooltip.js';

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
