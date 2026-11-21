// Browser-only DOM host. No protocol objects, credentials, or message data live here.
import '/public/ui.js';
import {loadPreferences, applyPreferences, setPreference} from './preferences.js';
import {swipeAction, installOverlayHistory, syncOverlayHistory} from './navigation.js';
const emit = (action) =>
  window.dispatchEvent(new CustomEvent("sticktalk-live", { detail: action }));
const $ = (s) => document.querySelector(s);
// Things that cannot be taken back ask first, in the app's own dialog.
const confirmations = [
  [/^delete:/, 'Delete this message?', 'This can’t be undone.', 'Delete'],
  [/^logout$/, 'Log out?', 'You’ll need your password to sign in on this device again.', 'Log Out'],
  [/^logout-others$/, 'Log out all other devices?', 'Every other device signed in to your account is signed out.', 'Log Out Devices'],
  [/^device-revoke:/, 'Log out that device?', 'It will have to sign in again.', 'Log Out'],
  [/^role-delete:/, 'Delete this role?', 'Members who have it lose its permissions. This can’t be undone.', 'Delete Role'],
  [/^friend:block,/, 'Block this person?', 'They won’t be able to message you or add you as a friend.', 'Block'],
  [/^friend:remove,/, 'Remove this friend?', 'You can add each other again later.', 'Remove Friend'],
  [/^leave-guild$/, 'Leave this server?', 'You’ll need a new invite to come back.', 'Leave Server'],
  [/^group-leave$/, 'Leave this group?', 'You won’t see new messages unless someone adds you again.', 'Leave Group'],
  [/^group-remove:/, 'Remove this person from the group?', '', 'Remove'],
  [/^submit:guild-icon-remove$/, 'Remove the server icon?', '', 'Remove Icon'],
  [/^avatar-remove$/, 'Remove your avatar?', '', 'Remove Avatar'],
];
// Actions that leave the page a settings form is on.
const leaving = /^(close$|home$|dialog:|channel:|guild:|voice:)/;
const unsaved = () => !!$('.settings-page form[data-live-form][data-dirty="true"],.live-modal form[data-live-form="edit"][data-dirty="true"]');
// An open question lives inside the open dialog, if there is one: that dialog
// treats presses anywhere else as "outside" and swallows them. A render replaces
// the dialog under it, so the question is put back after each one.
let asking = null;
function placeQuestion() {
  if (!asking || asking.isConnected) return;
  asking.removeAttribute('open');
  ($('.live-modal') ?? document.body).append(asking);
  asking.showModal();
}
function ask(title, body, label) {
  return new Promise((resolve) => {
    document.getElementById('live-confirm')?.remove();
    const dialog = document.createElement('dialog');
    dialog.id = 'live-confirm';
    dialog.className = 'confirm-dialog';
    dialog.setAttribute('aria-labelledby', 'live-confirm-title');
    const heading = document.createElement('h2');
    heading.id = 'live-confirm-title';
    heading.textContent = title;
    const copy = document.createElement('p');
    copy.textContent = body;
    copy.hidden = !body;
    const row = document.createElement('div');
    row.className = 'confirm-actions';
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.className = 'button secondary';
    cancel.textContent = 'Cancel';
    const accept = document.createElement('button');
    accept.type = 'button';
    accept.className = 'button danger';
    accept.textContent = label;
    accept.dataset.accept = '';
    row.append(cancel, accept);
    dialog.append(heading, copy, row);
    let answer = false;
    cancel.onclick = () => dialog.close();
    accept.onclick = () => { answer = true; dialog.close(); };
    dialog.addEventListener('click', (e) => { if (e.target === dialog) dialog.close(); });
    // Escape answers this question; it must not also close the page behind it.
    dialog.addEventListener('keydown', (e) => e.stopPropagation());
    // Removed after this click has finished: the dialog under it must still
    // see that the press landed here, not "outside" it.
    dialog.addEventListener('close', () => { asking = null; resolve(answer); setTimeout(() => dialog.remove(), 0); });
    asking = dialog;
    placeQuestion();
    cancel.focus();
  });
}
// Everything a person does by click, key or menu goes through here. Actions the
// app dispatches itself do not, so nothing is asked twice.
async function request(action) {
  const rule = confirmations.find(([pattern]) => pattern.test(action));
  if (rule && !(await ask(rule[1], rule[2], rule[3]))) return;
  if (leaving.test(action) && unsaved() &&
      !(await ask('Discard unsaved changes?', 'Your changes on this page haven’t been saved.', 'Discard'))) return;
  // Above the width where the member list is an overlay, its button shows or hides it.
  if (action === 'panel:right' && innerWidth > 1100) action = 'members-toggle';
  emit(action);
}
addEventListener('sticktalk-request', (e) => void request(e.detail));
// Several chosen, pasted or dropped files are sent to the app one at a time.
const uploads = [];
let uploading = false;
function queueUploads(files) {
  uploads.push(...files);
  if (!uploading) nextUpload();
}
function nextUpload() {
  const file = uploads.shift(), picker = $('#live-file');
  if (!file || !picker) { uploads.length = 0; uploading = false; return; }
  uploading = true;
  const transfer = new DataTransfer();
  transfer.items.add(file);
  picker.files = transfer.files;
  emit('upload');
  picker.value = '';
}
// The offline notice takes the connection banner's place in the layout, so it
// never covers the controls under it.
function network() {
  document.documentElement.dataset.offline = String(!navigator.onLine);
  let banner = $('#offline-banner');
  if (navigator.onLine) { banner?.remove(); return; }
  if (!banner) {
    banner = document.createElement('div');
    banner.id = 'offline-banner';
    banner.className = 'offline-banner';
    banner.setAttribute('role', 'status');
    banner.textContent = 'You’re offline. Your draft is kept.';
  }
  const header = $('.live-shell .channel-header');
  if (header) { if (header.nextElementSibling !== banner) header.after(banner); }
  else if (banner.parentElement !== document.body) document.body.append(banner);
}
function countMessage(input) {
  const counter = $('#live-count');
  if (!counter || !input) return;
  const length = [...input.value].length;
  counter.hidden = length < 3600;
  counter.textContent = String(4000 - length);
  counter.classList.toggle('is-over', length > 4000);
}
let caret, opener, focusComposer = false, toastText = '', toastTimer;
let saved,
  installed = false,
  composing = false,
  observer,
  seekTimer,
  pendingSeek,
  pendingHistoryJump,
  typingTimer,
  lastChannel,
  media;
let viewer;
let messageMenu;
let lastDialog;
let noticeTimer;
let rotatingPassword = false;
let finishPasswordRotation;
let sessionCleanup = Promise.resolve();
let authPending = false;
function probeSession() {
  if (rotatingPassword) return Promise.resolve(false);
  const probe = fetch('/api/v1/browser/session', {
    cache:'no-store', signal:AbortSignal.timeout(15000),
  }).then(response => response.status === 401).catch(() => false);
  // An old-cookie probe must finish before a new login can set a replacement
  // cookie, otherwise its late expiry response could erase that new session.
  sessionCleanup = Promise.all([sessionCleanup, probe]).then(() => {});
  return probe;
}
async function purgePrivateCaches() {
  // The probe expires a remotely revoked HttpOnly cookie at the server boundary.
  // It never restores a session or caches its response.
  void probeSession();
  navigator.serviceWorker?.controller?.postMessage('purge-private');
  if (!globalThis.caches) return;
  for (const name of await caches.keys()) {
    if (!name.startsWith('sticktalk-')) continue;
    if (!name.startsWith('sticktalk-shell-')) { await caches.delete(name); continue; }
    const cache = await caches.open(name);
    for (const request of await cache.keys()) {
      if (new URL(request.url).pathname.startsWith('/api/')) await cache.delete(request);
    }
  }
}
const openMessageMenu=async row=>{
  const id=typeof row==='string'?row:row.dataset.liveMessage;
  messageMenu??=await import('/public/message-menu.js');
  const current=[...document.querySelectorAll('[data-live-message]')].find(row=>row.dataset.liveMessage===id);
  if(current)messageMenu.openMessageMenu(current);
};
let lastSwipe;
let invite = new URLSearchParams(location.search).get('invite');
let inviteOpened = false;
let reauth = new URLSearchParams(location.search).get('reauth');
function updateJump(list) {
  if(!list)return;
  let button=$('#jump-to-present');
  if(!button) {
    button=document.createElement('button');button.id='jump-to-present';
    button.className='button secondary jump-to-present';button.textContent='Jump to present';
    button.onclick=()=>{
      const current=$('#live-messages');if(!current)return;
      current.scrollTop=current.scrollHeight;
      if(Number(current.dataset.windowEnd)<Number(current.dataset.total))emit('present');
    };
    $('.composer-wrap')?.append(button);
  }
  button.hidden=Number(list.dataset.windowEnd)>=Number(list.dataset.total)&&list.scrollHeight-list.scrollTop-list.clientHeight<list.clientHeight;
}
function resizeComposer(input) {
  if(!input)return;
  input.style.height='auto';
  input.style.height=`${Math.min(input.scrollHeight,Math.max(44,(window.visualViewport?.height??innerHeight)*.4))}px`;
}
const closeViewer = () => viewer?.closeViewer();
const openViewer = async (target) => {
  viewer ??= await import('/public/viewer.js');
  viewer.openViewer(target);
};
export async function directSearch(key, query) {
  return (await import('/public/gifs.js')).directSearch(key, query);
}
export async function recoverSession() {
  const pending = globalThis.__sticktalkSession;
  delete globalThis.__sticktalkSession;
  return pending ? await pending : '';
}
// No credentials or response DTOs cross this boundary. WebSocket's error event
// intentionally hides HTTP status; only a real 401 means the session was revoked.
export async function sessionRevoked() {
  return probeSession();
}
export function chatReady() {
  if (!performance.getEntriesByName('sticktalk-chat-ready').length)
    setTimeout(() => performance.mark('sticktalk-chat-ready'), 0);
}
let completionRange, completionRequest = '';
function complete(input) {
  const before = input.value.slice(0, input.selectionStart);
  const match = before.match(/(?:^|\s)([@#:])([^\s@#:]*)$/u);
  const request = match ? `${match[1]}|${match[2]}` : '';
  completionRange = match ? [input.selectionStart - match[2].length - 1, input.selectionStart] : null;
  if (request !== completionRequest) {
    completionRequest = request;
    emit(request ? `complete:${request}` : 'complete-close');
  }
}
function acceptCompletion(row) {
  const input = $('#live-input');
  if (!input || !completionRange) return;
  input.setRangeText(`${row.dataset.completion} `, ...completionRange, 'end');
  remember($('#live-messages').dataset.channel, input.value);
  completionRange = null;
  completionRequest = '';
  input.focus();
  emit('complete-close');
}
const drafts = new Map();
function remember(channel, text) {
  if (!drafts.has(channel) && drafts.size >= 64)
    drafts.delete(drafts.keys().next().value);
  drafts.set(channel, text.slice(0, 16000));
}
export function beforeRender() {
  const list = $("#live-messages"),
    input = $("#live-input"),
    focus = document.activeElement;
  if (input && list) remember(lastChannel ?? list.dataset.channel, input.value);
  const anchor = list && [...list.querySelectorAll('[data-live-message]')].find(row => row.getBoundingClientRect().bottom > list.getBoundingClientRect().top);
  saved = {
    channel: list?.dataset.channel,
    top: list?.scrollTop,
    bottom: list && list.scrollHeight - list.scrollTop - list.clientHeight < 96,
    anchor: anchor?.dataset.liveMessage,
    anchorOffset: anchor && anchor.getBoundingClientRect().top - list.getBoundingClientRect().top,
    id: focus?.id,
    start: focus?.selectionStart,
    end: focus?.selectionEnd,
    dialog: $(".live-modal")?.dataset.dialog,
    authUsername: $('#auth-username')?.value,
    completion: $('#live-completions [aria-selected="true"]')?.dataset.completion,
    fields: [
      ...document.querySelectorAll(
        ".live-modal input:not([type=file]),.live-modal select,.live-modal textarea",
      ),
    ].filter(e=>e.dataset.dirty==='true'||e.closest('form')?.dataset.dirty==='true').map((e) => {
      const selector = e.id ? `#${CSS.escape(e.id)}` :
        ['group-user','member-role','role-bit','settings-search'].map(key => e.hasAttribute(`data-${key}`) ? `[data-${key}="${CSS.escape(e.getAttribute(`data-${key}`))}"]` : null).find(Boolean);
      return [selector, e.value, e.type === 'checkbox' ? e.checked : null];
    }),
  };
}
let mediaQueue = Promise.resolve();
function withMedia(fn) {
  mediaQueue = mediaQueue
    .then(async () => {
      media ??= await import("/public/media.js");
      await fn(media);
    })
    .catch((e) => emit(`media-error:${e.message}`));
}
export function mediaEvent(raw) {
  withMedia((m) => m.event(JSON.parse(raw)));
}
export function mediaAction(action) {
  if (action === 'password-changing') {
    if (!rotatingPassword) {
      const rotation = new Promise(resolve => { finishPasswordRotation = resolve; });
      sessionCleanup = Promise.all([sessionCleanup, rotation]).then(() => {});
    }
    rotatingPassword = true;
    return;
  }
  if (action === 'password-finished') {
    rotatingPassword = false;
    const finish = finishPasswordRotation;
    finishPasswordRotation = undefined;
    if ($('#auth-username')) void probeSession().then(() => finish?.());
    else finish?.();
    return;
  }
  if (action === 'upload-done') { setTimeout(nextUpload, 0); return; }
  if (action === 'upload-stop') { uploads.length = 0; uploading = false; return; }
  if (action === 'form-saved') {
    // What is in the form is now what is saved.
    for (const form of document.querySelectorAll('.live-modal form[data-dirty]')) {
      delete form.dataset.dirty;
      for (const field of form.elements) delete field.dataset.dirty;
    }
    return;
  }
  if (action === 'profile-saved') {
    const form = $('[data-profile-editor]');
    if (form) {
      for (const input of form.querySelectorAll('input')) {
        input.defaultValue = input.value;
        delete input.dataset.dirty;
      }
      delete form.dataset.dirty;
      syncProfile();
    }
    return;
  }
  if(action.startsWith('history-jump:')) {
    const [channel,message]=action.slice(13).split(':');pendingHistoryJump={channel,message};return;
  }
  if (action.startsWith("purge-channel:")) {
    messageMenu?.closeMessageMenu();
    completionRequest = ''; completionRange = null;
    const channel = action.slice(14);
    drafts.delete(channel);
    if ($("#live-messages")?.dataset.channel === channel) {
      const input = $("#live-input");
      if (input) input.value = "";
      saved = null;
      composing = false;
    }
    if ($("#live-viewer")?.dataset.channel === channel) closeViewer();
    return;
  }

  if (action.startsWith("restore-draft:")) {
    const raw = action.slice(14),
      split = raw.indexOf(":"),
      channel = raw.slice(0, split),
      text = raw.slice(split + 1);
    const list = $("#live-messages"),
      input = $("#live-input");
    if (
      !drafts.get(channel) &&
      !(list?.dataset.channel === channel && input?.value)
    ) {
      remember(channel, text);
      if (list?.dataset.channel === channel && input) input.value = text;
    }
    return;
  }

  if (action.startsWith("sent:")) {
    const channel = action.slice(5);
    drafts.delete(channel);
    const list = $("#live-messages"),
      input = $("#live-input");
    if (list?.dataset.channel === channel) {
      if (input) input.value = "";
      if (saved) saved.bottom = true;
    }
    return;
  }
  if (action === "present") {
    clearTimeout(seekTimer);seekTimer=undefined;pendingSeek=undefined;
    const list = $("#live-messages");
    if (list) list.scrollTop = list.scrollHeight;
    if (saved) saved.bottom = true;
    return;
  }
  if (action === "purge") {
    void purgePrivateCaches();
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => emit('dismiss-notice'), 4000);
    messageMenu?.closeMessageMenu();
    drafts.clear();
    lastChannel=undefined;
    pendingHistoryJump=undefined;
    clearTimeout(seekTimer);seekTimer=undefined;pendingSeek=undefined;
    // A reactive render snapshots the old DOM before replacing it. Wipe the
    // live composer too, so that snapshot cannot resurrect an account draft.
    const input = $("#live-input");
    if (input) input.value = "";
    saved = null;
    composing = false;
    closeViewer();
    return;
  }
  if (action.startsWith("insert:")) {
    const input = $("#live-input");
    const c = $("#live-messages")?.dataset.channel;
    const base = input?.value ?? drafts.get(c) ?? "", added = action.slice(7);
    // Where the caret was when the picker opened, not always the end.
    const [start, end] = caret && caret[2] === c && caret[1] <= base.length ? caret : [base.length, base.length];
    const text = base.slice(0, start) + added + base.slice(end);
    caret = [start + added.length, start + added.length, c];
    if (c) remember(c, text);
    if (input) input.value = text;
    focusComposer = true;
    return;
  }
  if ((action === "transport-lost" || action === "reconnected") && !media)
    return;
  withMedia((m) => m.action(action));
}
function restore() {
  const list = $("#live-messages"),
    input = $("#live-input");
  if (list && input) {
    const changed=lastChannel!==list.dataset.channel;
    if(changed)composing=false;
    if (!composing)
      input.value = drafts.get(list.dataset.channel) ?? (changed?'':input.value);
    if (changed) {
      list.scrollTop = list.scrollHeight;
      lastChannel = list.dataset.channel;
    } else if (saved?.channel === list.dataset.channel) {
      list.scrollTop = saved.bottom ? list.scrollHeight : saved.top;
      const anchor = saved.anchor && list.querySelector(`[data-live-message="${saved.anchor}"]`);
      if (!saved.bottom && anchor)
        list.scrollTop += anchor.getBoundingClientRect().top - list.getBoundingClientRect().top - saved.anchorOffset;
    }
  }
  if (saved) {
    if(saved.authUsername && $('#auth-username'))$('#auth-username').value=saved.authUsername;
    if ($(".live-modal")?.dataset.dialog === saved.dialog)
      for (const [selector, value, checked] of saved.fields) {
        const e = selector && $('.live-modal')?.querySelector(selector);
        if (e) {
          e.value = value;
          if (checked !== null) e.checked = checked;
          e.dataset.dirty='true';
          if(e.form)e.form.dataset.dirty='true';
        }
      }
    if (saved.completion) {
      const rows = [...document.querySelectorAll('#live-completions [role=option]')];
      if (rows.some(row => row.dataset.completion === saved.completion))
        rows.forEach(row => row.setAttribute('aria-selected',String(row.dataset.completion === saved.completion)));
    }
    const focus = document.getElementById(saved.id);
    if (focus && !composing) {
      focus.focus({ preventScroll: true });
      if (saved.start != null && focus.type !== "color")
        try {
          focus.setSelectionRange(saved.start, saved.end);
        } catch {}
    }
    saved = null;
  }
  if(list&&pendingHistoryJump) {
    if(list.dataset.channel===pendingHistoryJump.channel) {
      const row=list.querySelector(`[data-live-message="${pendingHistoryJump.message}"]`);
      if(row){list.scrollTop+=row.getBoundingClientRect().top-list.getBoundingClientRect().top;pendingHistoryJump=undefined;}
    } else pendingHistoryJump=undefined;
  }
  if(list)list.__sticktalkScrollTop=list.scrollTop;
  const modal = $('.live-modal');
  const dialog = modal?.dataset.dialog;
  if (dialog !== lastDialog) {
    if (modal) {
      modal.scrollTop = 0;
      for (const pane of modal.querySelectorAll('.settings-main,.settings-nav')) pane.scrollTop = 0;
    } else if (lastDialog && !$('#live-confirm')) {
      // A closed dialog hands focus back to what opened it, or to the composer.
      const back = opener && [...document.querySelectorAll('.live-shell [data-live]')].find(e => e.dataset.live === opener && e.offsetParent);
      if (back) back.focus({preventScroll:true});
      else if (!matchMedia('(pointer:coarse)').matches) focusComposer = true;
    }
    lastDialog = dialog;
  }
  if (modal && !modal.contains(document.activeElement) && !$('#live-confirm')) modal.focus({preventScroll:true});
  if (focusComposer && !modal) {
    focusComposer = false;
    if (input) {
      input.focus({preventScroll:true});
      if (caret && caret[2] === list?.dataset.channel) try { input.setSelectionRange(caret[0], caret[1]); } catch {}
    }
  }
  placeQuestion();
  network();
  countMessage(input);
  // A toast is read and then gets out of the way.
  const toast = $('[data-toast]');
  const shown = toast ? `${toast.dataset.toast}:${toast.textContent}` : '';
  if (shown !== toastText) {
    toastText = shown;
    clearTimeout(toastTimer);
    if (toast) toastTimer = setTimeout(() => { toastText = ''; emit('dismiss-error'); }, toast.dataset.toast === 'notice' ? 4000 : 8000);
  }
  const shell=$('.live-shell');
  if(shell)shell.inert=!!modal||!!$('#live-viewer,#live-message-menu');
  const left=$('.left-panel');
  if(left)left.inert=innerWidth<768&&!shell?.classList.contains('panel-left');
  applyPreferences();
  resizeComposer(input);
  updateJump(list);
  syncOverlayHistory();
  if(reauth && $('.live-home') && !modal) {
    const page={password:'password',delete_account:'delete-account',link:'account',unlink:'account',linked:'account'}[reauth];
    reauth=null;if(page)emit(`dialog:${page}`);
  }
  if(invite && $('.live-home') && !modal && !inviteOpened) {
    inviteOpened=true; emit('dialog:join');
  } else if(invite && modal?.dataset.dialog==='join' && $('#f-name')) {
    $('#f-name').value=invite;$('#f-name').dataset.dirty='true';invite=null;
    // The code is in the form now. Keep it out of the address, or every reload asks again.
    const url=new URL(location.href);url.searchParams.delete('invite');history.replaceState(history.state,'',url);
  }
  media?.mount();
  syncProfile();
  syncRolePositions();
}
function syncProfile() {
  const form = $('[data-profile-editor]');
  if (!form) return;
  const inputs = [...form.querySelectorAll('input')];
  const dirty = inputs.some(input => input.value !== input.defaultValue);
  form.querySelector('.profile-save-bar').hidden = !dirty;
  form.dataset.dirty = String(dirty);
  for (const input of inputs) input.dataset.dirty = String(dirty);
  $('[data-preview-name]').textContent = form.querySelector('#f-name').value || form.dataset.fallbackName;
  $('[data-preview-status]').textContent = form.querySelector('#f-value').value;
}
let reorderingRoles = false;
function syncRolePositions() {
  const toggle = $('[data-role-reorder]');
  if (!toggle) { reorderingRoles = false; return; }
  toggle.textContent = reorderingRoles ? 'Done' : 'Reorder';
  toggle.setAttribute('aria-pressed', String(reorderingRoles));
  const selects = [...document.querySelectorAll('[data-role-position]')];
  selects.forEach((select, index) => {
    if (select.options.length !== selects.length) {
      select.replaceChildren(...selects.map((_, i) => new Option(`Position ${i + 1}`, String(i + 1))));
    }
    select.value = String(index + 1);
    select.hidden = !reorderingRoles;
  });
}
function send() {
  const input = $("#live-input");
  clearTimeout(typingTimer);
  completionRequest = ''; completionRange = null;
  if (input) emit(`send:${input.value}`);
}
export function installPlatform() {
  if (installed) return;
  installed = true;
  loadPreferences();
  installOverlayHistory(emit);
  addEventListener('sticktalk-close-surfaces',()=>{messageMenu?.closeMessageMenu(false);closeViewer();});
  addEventListener('sticktalk-surfaces-changed',()=>{syncOverlayHistory();const shell=$('.live-shell');if(shell)shell.inert=!!$('.live-modal,#live-viewer,#live-message-menu');});
  document.addEventListener("compositionstart", () => (composing = true));
  document.addEventListener("compositionend", () => (composing = false));
  observer = new MutationObserver((records) => {
    if (
      records.some(
        (r) =>
          (r.type === 'attributes' && (r.target.matches('.live-shell,[data-role-id]') || r.target.id==='sticktalk')) ||
          r.type === "childList" &&
          !r.target.closest?.('#live-media-stage,#live-device-settings,#live-viewer') &&
          [...r.addedNodes,...r.removedNodes].some(
            (n) =>
              n.nodeType === 1 &&
              (!n.closest ||
                !n.closest(
                  "#live-media-stage,#live-device-settings,#live-viewer",
                )),
          ),
      )
    )
      restore();
  });
  // Drawer toggles can update only a class (no child nodes). Without this,
  // the newly visible drawer stays inert and Back never receives its entry.
  observer.observe(document.body, { childList: true, subtree: true, attributes:true, attributeFilter:['class','data-screen','data-role-id'] });
  document.addEventListener("click", (e) => {
    if (e.target.closest('[data-profile-reset]')) {
      $('[data-profile-editor]').reset();
      syncProfile();
      return;
    }
    if (e.target.closest('[data-role-reorder]')) {
      reorderingRoles = !reorderingRoles;
      syncRolePositions();
      return;
    }
    if(lastSwipe&&performance.now()<lastSwipe.until&&Math.hypot(e.clientX-lastSwipe.x,e.clientY-lastSwipe.y)<32) {
      lastSwipe=null;e.preventDefault();return;
    }
    const completion = e.target.closest('[data-completion]');
    if (completion) { e.preventDefault(); acceptCompletion(completion); return; }
    // A visible button for a hidden file input, so the keyboard can reach it.
    const pick = e.target.closest('[data-pick]');
    if (pick) { e.preventDefault(); document.getElementById(pick.dataset.pick)?.click(); return; }
    const target = e.target.closest("[data-live]");
    if (target) {
        // Chromium may fat-finger-adjust a touch on message text to its nearby
        // author button. Only an actual hit on that button opens the profile.
        if(e.pointerType==='touch'&&target.matches('.message .author')) {
          const r=target.getBoundingClientRect();
          if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)return;
        }
      e.preventDefault();
      // Close the sheet and let its history entry go first. An action that opens
      // a dialog is then replayed after that entry is popped (navigation.js);
      // sent earlier, the pop would close the dialog it had just opened.
      if(target.closest('#live-message-menu'))messageMenu?.closeMessageMenu();
      const action = target.dataset.live;
      if (!target.closest('.live-modal,#live-message-menu')) opener = action;
      if (action.startsWith('reply:')) focusComposer = true;
      if (action === "pick-image") { emit("close"); $("#live-file")?.click(); }
      else if (action === "pick-file") $("#live-file")?.click();
      else if (action === "send-input") send();
      else if (action === "dismiss-error") emit("dismiss-error");
      else void request(action);
      return;
    }
    if (e.target.matches("[data-live-close]")) emit("close");
    const copy = e.target.closest("[data-copy]");
    if (copy) {
      e.preventDefault();
      if(!navigator.clipboard){emit('media-error:Clipboard unavailable. Select and copy the text instead.');return;}
      navigator.clipboard.writeText(copy.dataset.copy).then(()=>copy.textContent='Copied!').catch(()=>emit('media-error:Could not copy. Select and copy the text instead.'));
    }
    if (e.target.closest("[data-spoiler]"))
      e.target.closest("[data-spoiler]").classList.add("revealed");
    const image = e.target.closest("[data-view-image]");
    if (image) openViewer(image);
    if (e.target.closest("[data-push-enable]")) {
      if (!("Notification" in window) || !("PushManager" in window)) {
        emit(
          "media-error:Browser notifications are unavailable. On iOS, install StickTalk to your Home Screen first.",
        );
        return;
      }
      // Called in the actual user gesture, after the in-app explainer, never on login.
      const permission = Notification.requestPermission();
      import("/public/push.js")
        .then((m) => m.subscribe(permission))
        .catch((e) => emit(`media-error:${e.message}`));
    }
  });
  document.addEventListener("submit", (e) => {
    if (e.target.matches("[data-live-form]")) {
      e.preventDefault();
      const kind = e.target.dataset.liveForm;
      if (kind === 'auth') {
        if (authPending) return;
        authPending = true;
        e.target.querySelector('[type=submit]').disabled = true;
        void sessionCleanup.then(() => { authPending = false; emit('auth'); });
      }
      else emit(`submit:${kind}`);
    }
    if (e.target.matches("[data-gif-search]")) {
      e.preventDefault();
      emit(`gif-search:${$("#gif-query").value}`);
    }
  });
  document.addEventListener("input", (e) => {
    if(e.target.closest('.live-modal')) {
      e.target.dataset.dirty='true';
      if(e.target.form)e.target.form.dataset.dirty='true';
      // "Saved." describes the form as it was saved, not what is typed after.
      const note=e.target.form?.querySelector('.form-saved:not([hidden])');
      if(note){note.hidden=true;emit('saved-clear');}
    }
    if (e.target.closest('[data-profile-editor]')) syncProfile();
    if (e.target.matches('[data-settings-search]')) {
      const query = e.target.value.trim().toLowerCase();
      const rows = [...document.querySelectorAll('.settings-nav-row')];
      rows.forEach(row => row.hidden = !row.textContent.toLowerCase().includes(query));
      let none = $('.settings-search-empty');
      if (rows.every(row => row.hidden)) {
        if (!none) {
          none = document.createElement('p');
          none.className = 'settings-search-empty muted-copy';
          none.setAttribute('role', 'status');
          none.textContent = 'No settings match that search.';
          e.target.closest('label').after(none);
        }
      } else none?.remove();
    }
    if (e.target.id === "live-input") {
      resizeComposer(e.target);
      const c = $("#live-messages")?.dataset.channel;
      if (c) remember(c, e.target.value);
      if (!composing) complete(e.target);
      countMessage(e.target);
      // The app sends this at most once every few seconds.
      if (c && e.target.value) emit('typing');
    }
    if (e.target.matches("[data-emoji-query]")) {
      clearTimeout(typingTimer);
      const query = e.target.value;
      typingTimer = setTimeout(() => emit(`emoji-query:${query}`), 120);
    }
  });
  document.addEventListener("change", (e) => {
    if (e.target.matches('[data-role-position]')) {
      emit(`role-move:${e.target.dataset.rolePosition},${e.target.value}`);
    }
    if(e.target.closest('.live-modal')) {
      e.target.dataset.dirty='true';
      if(e.target.form)e.target.form.dataset.dirty='true';
    }
    if(e.target.matches('[data-preference]'))setPreference(e.target);
    if(e.target.matches('[data-notification-target]'))emit(`dialog:target-notifications:${e.target.value}`);
    if (e.target.id === "live-file") {queueUploads([...e.target.files]);e.target.value='';}
    if (e.target.id === 'live-avatar') {emit('avatar-upload');e.target.value='';}
    if (e.target.id === 'live-guild-icon') {emit('guild-icon-upload');e.target.value='';}
    if (e.target.dataset.groupUser) {
      const selected = [...document.querySelectorAll('[data-group-user]:checked')];
      if (selected.length > 15) { e.target.checked = false; return; }
      $('#f-value').value = selected.map(e => e.dataset.groupUser).join(',');
    }
    if (e.target.dataset.roleBit) {
      const field = $("#f-permissions"),
        bit = BigInt(e.target.dataset.roleBit),
        mask = BigInt(field.value);
      field.value = String(e.target.checked ? mask | bit : mask & ~bit);
    }
    if (e.target.dataset.memberRole) {
      $("#f-value").value = [
        ...document.querySelectorAll("[data-member-role]:checked"),
      ]
        .map((e) => e.dataset.memberRole)
        .join(",");
    }
    if(e.target.id==='f-name'&&e.target.form?.dataset.liveForm==='member-role') {
      const roles=new Set((e.target.selectedOptions[0]?.dataset.roleIds??'').split(','));
      document.querySelectorAll('[data-member-role]').forEach(box=>box.checked=roles.has(box.dataset.memberRole));
      $('#f-value').value=[...document.querySelectorAll('[data-member-role]:checked')].map(box=>box.dataset.memberRole).join(',');
    }
  });
  // A pasted screenshot or copied file becomes an attachment. Pasted text,
  // including text that carries a picture of itself, stays an ordinary paste.
  document.addEventListener("paste", (e) => {
    const file = e.clipboardData?.files[0];
    const picker = $("#live-file");
    if (!file || !picker || $(".live-modal") || e.clipboardData.getData("text/plain")) return;
    if (e.target.id !== "live-input" && e.target.closest?.("input,textarea,[contenteditable]")) return;
    e.preventDefault();
    queueUploads([file]);
  });
  // Files dragged onto the conversation are attached, like chosen ones.
  const dragged = (e) => !!$('#live-file') && !$('.live-modal') && [...(e.dataTransfer?.types ?? [])].includes('Files');
  document.addEventListener('dragover', (e) => {
    if (!dragged(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    $('.main-pane')?.classList.add('is-dropping');
  });
  document.addEventListener('dragleave', (e) => { if (!e.relatedTarget) $('.main-pane')?.classList.remove('is-dropping'); });
  document.addEventListener('drop', (e) => {
    $('.main-pane')?.classList.remove('is-dropping');
    if (!dragged(e)) return;
    e.preventDefault();
    queueUploads([...e.dataTransfer.files].slice(0, 4));
  });
  document.addEventListener('focusout', (e) => {
    if (e.target.id === 'live-input') caret = [e.target.selectionStart, e.target.selectionEnd, $('#live-messages')?.dataset.channel];
  });
  // A picture that failed to load (a busy server) is asked for again, a few times.
  document.addEventListener('error', (e) => {
    const img = e.target;
    if (img.tagName !== 'IMG' || !img.src.includes('/api/v1/files/')) return;
    const tries = Number(img.dataset.retry ?? 0);
    if (tries >= 3) return;
    img.dataset.retry = String(tries + 1);
    setTimeout(() => {
      if (!img.isConnected) return;
      const url = new URL(img.src);
      url.searchParams.set('retry', String(tries + 1));
      img.removeAttribute('srcset');
      img.src = url;
    }, 1500 * (tries + 1));
  }, true);
  addEventListener('resize', () => {
    // The drawer is only a drawer on a phone; a widened window gets its sidebar back.
    const left = $('.left-panel');
    if (left) left.inert = innerWidth < 768 && !$('.live-shell')?.classList.contains('panel-left');
  });
  try {
    const notice = sessionStorage.getItem('sticktalk-notice');
    if (notice) {
      sessionStorage.removeItem('sticktalk-notice');
      const toast = document.createElement('div');
      toast.className = 'update-toast';
      toast.setAttribute('role', 'status');
      toast.textContent = notice.slice(0, 200);
      document.body.append(toast);
      setTimeout(() => toast.remove(), 6000);
    }
  } catch { /* Storage may be disabled. */ }
  document.addEventListener("keydown", (e) => {
    if(e.key==='ContextMenu'||(e.shiftKey&&e.key==='F10')||(e.key==='Enter'&&e.target.matches('.message'))) {
      const row=e.target.closest('.message');if(row){e.preventDefault();openMessageMenu(row);return;}
    }
    if (!e.isComposing && e.target.id === 'live-input') {
      const rows = [...document.querySelectorAll('#live-completions [role=option]')];
      if (rows.length && ['ArrowUp','ArrowDown','Tab','Enter','Escape'].includes(e.key)) {
        e.preventDefault();
        const selected = Math.max(0, rows.findIndex(r => r.getAttribute('aria-selected') === 'true'));
        if (e.key === 'Escape') { completionRequest = ''; completionRange = null; emit('complete-close'); }
        else if (e.key === 'Tab' || e.key === 'Enter') acceptCompletion(rows[selected]);
        else {
          const index = (selected + (e.key === 'ArrowUp' ? -1 : 1) + rows.length) % rows.length;
          rows.forEach((r,i) => r.setAttribute('aria-selected', String(i === index)));
          e.target.setAttribute('aria-activedescendant', rows[index].id);
        }
        return;
      }
    }
    if (e.key === "Escape") {
      if ($("#live-viewer")) closeViewer();
      else if (e.target.matches?.('[data-settings-search]') && e.target.value) {
        e.target.value = '';
        e.target.dispatchEvent(new Event('input', {bubbles:true}));
      }
      else if (!$('.live-modal') && $('.composer-wrap .reply-preview')) emit('cancel-reply');
      else void request("close");
    }
    // Editing a message: Enter saves, like sending one.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.target.id === 'f-name' && e.target.form?.dataset.liveForm === 'edit') {
      e.preventDefault();
      e.target.form.requestSubmit();
      return;
    }
    if ((e.key === 'Home' || e.key === 'End') && e.target.id === 'live-messages') {
      // The list shows a window of the history: go to its real start or end.
      const list = e.target;
      e.preventDefault();
      if (e.key === 'End') {
        list.scrollTop = list.scrollHeight;
        if (Number(list.dataset.windowEnd) < Number(list.dataset.total)) emit('present');
      } else if (Number(list.dataset.windowStart) > 0) emit('seek:0');
      else list.scrollTop = 0;
      return;
    }
    if (
      e.target.id === "live-input" &&
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.isComposing
    ) {
      e.preventDefault();
      send();
    }
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "m") {
      e.preventDefault();
      mediaAction("mute");
    }
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "d") {
      e.preventDefault();
      mediaAction("deafen");
    }
  });
  document.addEventListener(
    "scroll",
    (e) => {
      const list = e.target;
      if (list.id !== "live-messages") return;
      updateJump(list);
      const start = Number(list.dataset.windowStart),
        end = Number(list.dataset.windowEnd),
        total = Number(list.dataset.total);
      const previous=list.__sticktalkScrollTop??list.scrollTop;
      const direction=Math.sign(list.scrollTop-previous);list.__sticktalkScrollTop=list.scrollTop;
      const rows=[...list.querySelectorAll('[data-live-message]')];
      const top=list.getBoundingClientRect().top, first=rows[0],last=rows.at(-1);
      let action;
      if(first&&first.getBoundingClientRect().top>top+32&&start>0)
        action=first.getBoundingClientRect().top-top<list.clientHeight*2?'history-older':`seek:${Math.min(start-1,Math.max(0,Math.floor(list.scrollTop/88)))}`;
      else if(last&&last.getBoundingClientRect().bottom<top+list.clientHeight-32&&end<total)
        action=top-last.getBoundingClientRect().bottom<list.clientHeight*2?'history-newer':`seek:${Math.min(total-1,end+Math.floor(Math.max(0,top-last.getBoundingClientRect().bottom)/88))}`;
      else {
        const firstVisible=rows.findIndex(row=>row.getBoundingClientRect().bottom>top);
        const lastVisible=rows.findLastIndex(row=>row.getBoundingClientRect().top<top+list.clientHeight);
        if(direction<0&&firstVisible<10&&start>0)action='history-older';
        else if(direction>0&&firstVisible>=10&&lastVisible>=rows.length-10&&end<total)action='history-newer';
      }
      if(action&&total>50) {
        const {channel,historyGeneration,windowStart,windowEnd}=list.dataset;
        pendingSeek={action,channel,historyGeneration,windowStart,windowEnd};
        // Throttle, do not debounce: continuous momentum scrolling must load
        // adjacent windows too, not wait for the finger/inertia to stop.
        if(seekTimer===undefined)seekTimer=setTimeout(()=>{
          seekTimer=undefined;
          const request=pendingSeek;pendingSeek=undefined;
          const current=$('#live-messages');
          if(request&&current?.dataset.channel===request.channel&&current.dataset.historyGeneration===request.historyGeneration&&current.dataset.windowStart===request.windowStart&&current.dataset.windowEnd===request.windowEnd)emit(request.action);
        },80);
      }
      if (list.scrollHeight - list.scrollTop - list.clientHeight < 80)
        emit("read");
    },
    true,
  );
  const focus = () => {
    // Mobile documents may be frozen while connectivity changes. Reconcile on
    // foreground/pageshow too, without an idle polling loop.
    const wasOffline=document.documentElement.dataset.offline==='true';
    network();
    if(wasOffline&&navigator.onLine)emit('recover');
    emit(`focus:${!document.hidden && document.hasFocus()}`);
  };
  addEventListener('online',network);addEventListener('offline',network);network();
  window.addEventListener("online", () => emit("recover"));
  window.addEventListener('pageshow',focus);
  window.addEventListener("focus", focus);
  window.addEventListener("blur", focus);
  document.addEventListener("visibilitychange", focus);
  window.addEventListener("pagehide", () => {
    // The closing WebSocket already releases the SFU session. Do not signal
    // Leave here: it can start a lazy WASM fetch during document teardown,
    // which WebKit rejects with an unhandled access-control error.
    media?.action("transport-lost");
    emit("focus:false");
  });
  const viewport = () => {
    document.documentElement.style.setProperty(
      "--viewport-height",
      `${window.visualViewport?.height ?? innerHeight}px`,
    );
    document.documentElement.style.setProperty(
      "--viewport-offset",
      `${window.visualViewport?.offsetTop ?? 0}px`,
    );
  };
  window.visualViewport?.addEventListener("resize", viewport);
  window.visualViewport?.addEventListener("scroll", viewport);
  viewport();
  let touch, press;
  const cancelPress=()=>{if(press)clearTimeout(press.timer);press=null;};
  document.addEventListener('contextmenu',e=>{
    const row=e.target.closest('.message');
    if(!row||row.closest('.live-modal'))return;
    // With a mouse, a link, a picture or selected text keeps the browser's own menu.
    const touch=innerWidth<768||matchMedia('(pointer:coarse)').matches;
    if(!touch&&(e.target.closest('a,img,video,input,textarea')||String(getSelection()).trim()))return;
    e.preventDefault();cancelPress();if(!$('#live-message-menu'))openMessageMenu(row);
  });
  document.addEventListener("pointerdown", (e) => {
    cancelPress();
    if(e.target.closest('#live-message-menu'))lastSwipe=undefined;
    const hit=e.pointerType==='touch'?document.elementFromPoint(e.clientX,e.clientY)??e.target:e.target;
    const row=hit.closest('.message');
    if(e.pointerType==='touch'&&row&&!hit.closest('button,a,input,textarea,video')) {
      const messageId=row.dataset.liveMessage;
      press={x:e.clientX,y:e.clientY,timer:setTimeout(()=>{lastSwipe={x:e.clientX,y:e.clientY,until:performance.now()+750};openMessageMenu(messageId);press=null;},450)};
    }
    if (
      e.pointerType === "touch" &&
      innerWidth < 768 &&
      !e.target.closest("input,textarea,[role=dialog],dialog")
    )
      touch = { x: e.clientX, y: e.clientY, id:e.pointerId };
  });
  document.addEventListener('pointermove',e=>{if(press&&Math.hypot(e.clientX-press.x,e.clientY-press.y)>12)cancelPress();});
  document.addEventListener('pointercancel',()=>{touch=null;cancelPress();});
  document.addEventListener("pointerup", (e) => {
    cancelPress();
    if (touch && e.pointerId===touch.id) {
      const dx = e.clientX - touch.x,
        dy = e.clientY - touch.y;
      const shell=$('.live-shell');
      const panel=shell?.classList.contains('panel-left')?'left':shell?.classList.contains('panel-right')?'right':'';
      const action=swipeAction(panel,dx,dy);
      if(action){lastSwipe={x:e.clientX,y:e.clientY,until:performance.now()+500};emit(action);}
      touch = null;
    }
  });
  if ("serviceWorker" in navigator) {
    const register = () =>
      navigator.serviceWorker
        .register("/sw.js", { scope: "/" })
        .then(registration => {
          const offer = () => {
            if (!registration.waiting || registration.waiting.state!=='installed' || !registration.active ||
                registration.active.state!=='activated' || registration.waiting===registration.active ||
                registration.waiting===navigator.serviceWorker.controller || !navigator.serviceWorker.controller || $('#app-update')) return;
            const toast = document.createElement('div'); toast.id = 'app-update'; toast.className = 'update-toast';
            const text = document.createElement('span'); text.textContent = 'Update ready.';
            const reload = document.createElement('button'); reload.className = 'button secondary'; reload.textContent = 'Reload';
            reload.onclick = () => {
              if ($('#live-input')?.value || [...drafts.values()].some(Boolean) || $('.live-upload,#live-connected:not([hidden])') || $('.live-modal form[data-dirty="true"]')) {text.textContent = 'Finish your drafts, uploads or call before reloading.'; return;}
              navigator.serviceWorker.addEventListener('controllerchange', () => location.reload(), {once:true});
              registration.waiting?.postMessage('activate-update');
            };
            toast.append(text,reload); document.body.append(toast);
          };
          offer();
          registration.addEventListener('updatefound', () => registration.installing?.addEventListener('statechange',offer));
        })
        .catch(() => {});
    if ("requestIdleCallback" in window)
      requestIdleCallback(register, { timeout: 2000 });
    else setTimeout(register, 0);
  }
}
