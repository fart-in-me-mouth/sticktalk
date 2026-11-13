// Platform-only adapter. UI and content live in Rust view models, not this file.
// Event-driven: no polling, animation frames, notification/audio/browser APIs.
const send = (action) =>
  window.dispatchEvent(new CustomEvent("sticktalk-action", { detail: action }));
const $ = (selector) => document.querySelector(selector);
let lastAnchor = null,
  restoreScroll = null,
  toastTimer,
  longPressTimer,
  touch,
  suppressClick = false,
  offlineNotified = false;
const draft = new Map();
let workerStarted = false;

function dispatch(action, anchor) {
  if (anchor) {
    const r = anchor.getBoundingClientRect();
    lastAnchor = {
      x: anchor.closest(".member-list") ? r.left - 348 : r.left,
      y: r.bottom + 8,
    };
  }
  const messages = $("#messages");
  restoreScroll = messages
    ? {
        top: messages.scrollTop,
        bottom:
          messages.scrollHeight - messages.scrollTop - messages.clientHeight <
          80,
      }
    : null;
  const input = $("#message-input");
  if (input) draft.set(input.placeholder, input.value);
  send(action);
}

function viewport() {
  const m = $("#messages"),
    atBottom = m && m.scrollHeight - m.scrollTop - m.clientHeight < 80;
  const vv = window.visualViewport;
  const height = vv?.height ?? window.innerHeight;
  document.documentElement.style.setProperty(
    "--viewport-height",
    `${height}px`,
  );
  document.documentElement.style.setProperty(
    "--viewport-offset",
    `${vv?.offsetTop ?? 0}px`,
  );
  document.body.style.top = `${vv?.offsetTop ?? 0}px`;
  document.documentElement.classList.toggle(
    "has-keyboard",
    window.innerHeight - height > 120,
  );
  if (atBottom) m.scrollTop = m.scrollHeight;
}
window.visualViewport?.addEventListener("resize", viewport);
window.visualViewport?.addEventListener("scroll", viewport);
window.addEventListener("resize", viewport);
viewport();

function messageSend() {
  const input = $("#message-input");
  if (input?.value.trim()) {
    const text = input.value;
    input.value = "";
    draft.delete(input.placeholder);
    dispatch(`send:${text}`);
    restoreScroll = { bottom: true, top: 0 };
  }
}

document.addEventListener("click", async (event) => {
  if (suppressClick) {
    suppressClick = false;
    event.preventDefault();
    return;
  }
  const target = event.target instanceof Element ? event.target : null;
  if (!target) return;
  const action = target.closest("[data-action]");
  if (action) {
    const value = action.dataset.action;
    if (value === "toggle-local") {
      const checked = action.getAttribute("aria-checked") !== "true";
      action.setAttribute("aria-checked", String(checked));
      action.querySelector(".toggle")?.classList.toggle("on", checked);
      return;
    }
    if (value === "toggle-settings-nav") {
      $(".settings-nav")?.classList.toggle("open");
      return;
    }
    if (value === "remove-attachment") {
      $(".pending-attachment").hidden = true;
      return;
    }
    if (value === "pick-file") {
      // Local file chooser is intentionally not opened in fixture mode.
      dispatch("close");
      queueMicrotask(() => {
        $(".pending-attachment")?.removeAttribute("hidden");
      });
      return;
    }
    if (value === "fullscreen") {
      if (!document.fullscreenElement)
        await $(".stream-stage")?.requestFullscreen?.();
      else await document.exitFullscreen?.();
      return;
    }
    if (value === "copy") {
      // Clipboard requires an explicit user gesture, never invoked by tests or on load.
      if (event.isTrusted)
        navigator.clipboard
          ?.writeText(
            $(".invite-code code")?.textContent ?? "StickTalk message",
          )
          .catch(() => {});
    }
    if (action.hasAttribute("data-confirm") && action.disabled) return;
    dispatch(value, action);
    return;
  }
  if (target.closest("[data-send]")) {
    messageSend();
    return;
  }
  if (target.closest("[data-jump]")) {
    const m = $("#messages");
    m.scrollTop = m.scrollHeight;
    return;
  }
  if (target.matches("[data-backdrop]")) {
    dispatch("close");
    return;
  }
  const collapse = target.closest("[data-collapse]");
  if (collapse) {
    const el = document.getElementById(collapse.dataset.collapse);
    el.hidden = !el.hidden;
    collapse.setAttribute("aria-expanded", String(!el.hidden));
    return;
  }
  if (target.matches("[data-banner]")) {
    document
      .querySelectorAll(".banner-swatch")
      .forEach((e) => e.classList.toggle("selected", e === target));
    const p = $(".server-preview-banner");
    if (p) p.style.background = getComputedStyle(target).background;
    const mark = $(".settings-main");
    mark?.setAttribute("data-dirty-local", "true");
    dispatch("dirty");
    return;
  }
  if (target.matches("[data-reveal]")) {
    target.parentElement.firstChild.textContent = "alex.chen@gmail.com ";
    target.textContent = "Hide";
    return;
  }
  if (target.closest("[data-check-menu]")) {
    const e = target.closest("[data-check-menu]");
    e.setAttribute(
      "aria-checked",
      String(e.getAttribute("aria-checked") !== "true"),
    );
    return;
  }
  if (target.closest("[data-clear-permissions]")) {
    target
      .closest(".permission-section")
      .querySelectorAll("[role=switch][aria-checked=true]")
      .forEach((e) => dispatch(e.dataset.action));
  }
});

document.addEventListener("input", (event) => {
  const e = event.target;
  if (e.matches("[data-filter]")) {
    const q = e.value.toLowerCase();
    document
      .getElementById(e.dataset.filter)
      ?.querySelectorAll("[data-filter-row]")
      .forEach((row) => {
        const text = (
          row.textContent +
          " " +
          row.getAttribute("aria-label")
        ).toLowerCase();
        let i = 0;
        for (const c of text) {
          if (c === q[i]) i++;
        }
        row.hidden = i < q.length;
      });
  }
  if (e.id === "message-input") {
    e.style.height = "28px";
    e.style.height = `${Math.min(e.scrollHeight, window.innerHeight * 0.4)}px`;
    draft.set(e.placeholder, e.value);
  }
  if (e.hasAttribute("data-confirm-server")) {
    $("[data-confirm]").disabled = e.value !== e.dataset.confirmServer;
  }
});
document.addEventListener("change", (event) => {
  const e = event.target;
  if (e.dataset.changeAction) dispatch(`${e.dataset.changeAction}:${e.value}`);
  else if (e.hasAttribute("data-dirty")) dispatch("dirty");
});
document.addEventListener("submit", (event) => {
  if (event.target.matches("[data-auth]")) {
    event.preventDefault();
    const p = $("#password"),
      u = $("#username");
    if (!p.value || !u.value) {
      $("#auth-error").hidden = false;
      (u.value ? p : u).focus();
      return;
    }
    dispatch("screen:home");
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    dispatch("close");
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    dispatch("modal:quick");
    return;
  }
  if (
    event.ctrlKey &&
    event.shiftKey &&
    ["m", "d"].includes(event.key.toLowerCase())
  ) {
    event.preventDefault();
    dispatch(event.key.toLowerCase() === "m" ? "mute" : "deafen");
    return;
  }
  if (event.altKey && ["ArrowUp", "ArrowDown"].includes(event.key)) {
    event.preventDefault();
    const names = [
      "general",
      "introductions",
      "announcements",
      "projects",
      "screenshots",
    ];
    const current =
      $(".channel-row.selected .channel-main span")?.textContent ?? "general";
    const i = names.indexOf(current);
    dispatch(`channel:${names[(i + (event.key === "ArrowDown" ? 1 : 4)) % 5]}`);
    return;
  }
  if (
    event.target.id === "message-input" &&
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.isComposing
  ) {
    event.preventDefault();
    messageSend();
    return;
  }
  if (event.target.matches("[data-profile-message]") && event.key === "Enter") {
    event.preventDefault();
    dispatch(`send:${event.target.value}`);
    dispatch("screen:dm");
    return;
  }
  const dialog = $("[role=dialog]");
  if (dialog && event.key === "Tab") {
    const focusable = [
      ...dialog.querySelectorAll(
        'button:not(:disabled),input,textarea,select,[tabindex="0"]',
      ),
    ].filter((e) => !e.hidden && e.getClientRects().length);
    const first = focusable[0],
      last = focusable.at(-1);
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first?.focus();
    }
  }
  const quick = $("#quick-results");
  if (quick && ["ArrowDown", "ArrowUp", "Enter"].includes(event.key)) {
    const rows = [...quick.querySelectorAll("button:not([hidden])")];
    let i = rows.indexOf(document.activeElement);
    if (event.key === "Enter") {
      if (i < 0) rows[0]?.click();
    } else {
      event.preventDefault();
      rows[
        (i + (event.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length
      ]?.focus();
    }
  }
});

document.addEventListener("contextmenu", (event) => {
  const e = event.target.closest("[data-context],.server-icon,.server-rail,a");
  if (e) {
    event.preventDefault();
    lastAnchor = { x: event.clientX, y: event.clientY };
    dispatch(
      window.innerWidth < 768 && e.dataset.context === "message"
        ? `message-sheet:${e.dataset.message ?? 0}`
        : `menu:${e.dataset.context ?? "server"}`,
    );
  }
});

// Pointer-driven gestures only update CSS transforms. Release uses one CSS transition.
document.addEventListener("pointerdown", (event) => {
  if (event.pointerType !== "touch" || window.innerWidth >= 768) return;
  const shell = $(".app-shell");
  if (!shell) return;
  touch = {
    x: event.clientX,
    y: event.clientY,
    time: event.timeStamp,
    dx: 0,
    shell,
    moved: false,
    panel: shell.classList.contains("panel-left")
      ? "left"
      : shell.classList.contains("panel-right")
        ? "right"
        : "",
  };
  const message = event.target.closest("[data-message]");
  if (message) {
    longPressTimer = setTimeout(() => {
      if (touch && !touch.moved) {
        touch = null;
        suppressClick = true;
        dispatch(`message-sheet:${message.dataset.message}`);
      }
    }, 500);
  }
});
document.addEventListener(
  "pointermove",
  (event) => {
    if (!touch) return;
    const dx = event.clientX - touch.x,
      dy = event.clientY - touch.y;
    if (Math.abs(dx) > 8 || Math.abs(dy) > 8) {
      clearTimeout(longPressTimer);
      touch.moved = true;
    }
    if (Math.abs(dy) > Math.abs(dx) && !touch.horizontal) return;
    if (Math.abs(dx) < 12 && !touch.horizontal) return;
    if (event.target.closest("textarea,input,[role=dialog]")) return;
    touch.horizontal = true;
    touch.dx = dx;
    touch.shell.classList.add("gesture");
    const leftWidth = touch.shell
      .querySelector(".left-panel")
      .getBoundingClientRect().width;
    const offset =
      touch.panel === "left" ? leftWidth : touch.panel === "right" ? -300 : 0;
    const shift = Math.max(-300, Math.min(leftWidth, dx + offset));
    touch.shell.querySelector(".main-pane").style.transform =
      `translateX(${shift}px)`;
    touch.shell.querySelector(".left-panel").style.transform =
      `translateX(${Math.min(0, shift - leftWidth)}px)`;
    const members = touch.shell.querySelector(".member-list");
    if (members)
      members.style.transform = `translateX(${Math.max(0, 300 + shift)}px)`;
  },
  { passive: true },
);
function release(event) {
  clearTimeout(longPressTimer);
  if (!touch) return;
  const t = touch;
  touch = null;
  if (!t.horizontal) return;
  t.shell.classList.remove("gesture");
  t.shell
    .querySelectorAll(".main-pane,.left-panel,.member-list")
    .forEach((e) => e.style.removeProperty("transform"));
  const fast = Math.abs(t.dx) / Math.max(1, event.timeStamp - t.time) > 0.35;
  if (Math.abs(t.dx) > 80 || fast) {
    if (t.panel) {
      dispatch("close");
    } else {
      dispatch(`panel:${t.dx > 0 ? "left" : "right"}`);
    }
  } else if (!t.panel) {
    dispatch("close");
  }
}
document.addEventListener("pointerup", release);
document.addEventListener("pointercancel", () => {
  clearTimeout(longPressTimer);
  if (touch) {
    touch.shell.classList.remove("gesture");
    touch.shell
      .querySelectorAll(".main-pane,.left-panel,.member-list")
      .forEach((e) => e.style.removeProperty("transform"));
    touch = null;
  }
});

let dragged;
document.addEventListener("dragstart", (e) => {
  const r = e.target.closest("[data-role-index]");
  if (r) {
    dragged = Number(r.dataset.roleIndex);
    e.dataTransfer.setData("text/plain", String(dragged));
    r.classList.add("dragging");
  }
});
document.addEventListener("dragover", (e) => {
  const r = e.target.closest("[data-role-index]");
  if (r) {
    e.preventDefault();
    r.classList.add("drag-over");
  } else if (e.dataTransfer?.types.includes("Files") && $(".chat-body")) {
    e.preventDefault();
    $(".chat-body").classList.add("file-drop");
  }
});
document.addEventListener("dragleave", (e) =>
  e.target.closest("[data-role-index]")?.classList.remove("drag-over"),
);
document.addEventListener("drop", (e) => {
  const r = e.target.closest("[data-role-index]");
  if (r && dragged !== undefined) {
    e.preventDefault();
    dispatch(`reorder:${dragged},${r.dataset.roleIndex}`);
  } else if (e.dataTransfer?.files.length) {
    e.preventDefault();
    $(".pending-attachment")?.removeAttribute("hidden");
  }
  $(".chat-body")?.classList.remove("file-drop");
  dragged = undefined;
});
document.addEventListener("dragend", () =>
  document
    .querySelectorAll(".dragging,.drag-over")
    .forEach((e) => e.classList.remove("dragging", "drag-over")),
);

const initialized = new WeakSet();
function initialize() {
  const root = $("#sticktalk");
  if (root && !initialized.has(root)) {
    initialized.add(root);
    // Do not compete with the critical wasm/bootstrap fetch. One idle task,
    // only after the first view mounts; this is not a polling/animation loop.
    if (!workerStarted && "serviceWorker" in navigator) {
      workerStarted = true;
      const register = () =>
        navigator.serviceWorker
          .register("/sw.js", { scope: "/" })
          .catch(() => {});
      if ("requestIdleCallback" in window)
        window.requestIdleCallback(register, { timeout: 2000 });
      else setTimeout(register, 0);
    }
    if (!navigator.onLine && !offlineNotified) {
      offlineNotified = true;
      queueMicrotask(() => send("offline:true"));
    }
  }
  const m = $("#messages");
  if (m && !initialized.has(m)) {
    initialized.add(m);
    m.scrollTop =
      restoreScroll && !restoreScroll.bottom
        ? restoreScroll.top
        : m.scrollHeight;
    m.dispatchEvent(new Event("scroll"));
    m.addEventListener(
      "scroll",
      () => {
        $("[data-jump]")?.classList.toggle(
          "visible",
          m.scrollHeight - m.scrollTop - m.clientHeight > m.clientHeight,
        );
      },
      { passive: true },
    );
    restoreScroll = null;
  }
  const input = $("#message-input");
  if (input && !initialized.has(input)) {
    initialized.add(input);
    input.value = draft.get(input.placeholder) ?? "";
  }
  const floating = $("[data-floating]");
  if (floating && !initialized.has(floating)) {
    initialized.add(floating);
    if (
      (lastAnchor || !floating.matches(".profile-popout")) &&
      (window.innerWidth >= 768 || !floating.matches(".profile-popout"))
    ) {
      const r = floating.getBoundingClientRect(),
        anchor = lastAnchor ?? { x: 80, y: 56 };
      floating.style.left = `${Math.max(8, Math.min(anchor.x, window.innerWidth - r.width - 8))}px`;
      floating.style.top = `${Math.max(8, Math.min(anchor.y, window.innerHeight - r.height - 8))}px`;
      floating.style.right = "auto";
    }
  }
  const modal = $(".modal");
  if (modal && !initialized.has(modal)) {
    initialized.add(modal);
    queueMicrotask(() => {
      (modal.querySelector("input") ?? modal).focus({ preventScroll: true });
    });
  }
  const toast = $(".toast");
  if (toast && !initialized.has(toast)) {
    initialized.add(toast);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => dispatch("clear-toast"), 3500);
  }
}
new MutationObserver(initialize).observe(document.body, {
  childList: true,
  subtree: true,
});
initialize();
window.addEventListener("offline", () => {
  offlineNotified = true;
  dispatch("offline:true");
});
window.addEventListener("online", () => {
  offlineNotified = false;
  dispatch("offline:false");
});
if (!navigator.onLine) dispatch("offline:true");
