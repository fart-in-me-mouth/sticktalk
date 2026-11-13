// Browser-only DOM host. No protocol objects, credentials, or message data live here.
const emit = (action) =>
  window.dispatchEvent(new CustomEvent("sticktalk-live", { detail: action }));
const $ = (s) => document.querySelector(s);
let saved,
  installed = false,
  composing = false,
  observer,
  seekTimer,
  typingTimer,
  lastChannel,
  media;
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
  if (input && list) remember(list.dataset.channel, input.value);
  saved = {
    channel: list?.dataset.channel,
    top: list?.scrollTop,
    bottom: list && list.scrollHeight - list.scrollTop - list.clientHeight < 96,
    id: focus?.id,
    start: focus?.selectionStart,
    end: focus?.selectionEnd,
    dialog: $(".live-modal")?.dataset.dialog,
    fields: [
      ...document.querySelectorAll(
        ".live-modal input:not([type=password]),.live-modal select,.live-modal textarea",
      ),
    ].map((e) => [e.id, e.value, e.type === "checkbox" ? e.checked : null]),
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
  if (action.startsWith("purge-channel:")) {
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
    const list = $("#live-messages");
    if (list) list.scrollTop = list.scrollHeight;
    if (saved) saved.bottom = true;
    return;
  }
  if (action === "purge") {
    drafts.clear();
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
    const text = (input?.value ?? drafts.get(c) ?? "") + action.slice(7);
    if (c) remember(c, text);
    if (input) input.value = text;
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
    if (!composing)
      input.value = drafts.get(list.dataset.channel) ?? input.value;
    if (lastChannel !== list.dataset.channel) {
      list.scrollTop = list.scrollHeight;
      lastChannel = list.dataset.channel;
    } else if (saved?.channel === list.dataset.channel) {
      list.scrollTop = saved.bottom ? list.scrollHeight : saved.top;
    }
  }
  if (saved) {
    if ($(".live-modal")?.dataset.dialog === saved.dialog)
      for (const [id, value, checked] of saved.fields) {
        const e = document.getElementById(id);
        if (e) {
          e.value = value;
          if (checked !== null) e.checked = checked;
        }
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
  media?.mount();
}
function send() {
  const input = $("#live-input");
  if (input) emit(`send:${input.value}`);
}
export function installPlatform() {
  if (installed) return;
  installed = true;
  document.addEventListener("compositionstart", () => (composing = true));
  document.addEventListener("compositionend", () => (composing = false));
  observer = new MutationObserver((records) => {
    if (
      records.some(
        (r) =>
          r.type === "childList" &&
          [...r.addedNodes].some(
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
  observer.observe(document.body, { childList: true, subtree: true });
  document.addEventListener("click", (e) => {
    const target = e.target.closest("[data-live]");
    if (target) {
      e.preventDefault();
      const action = target.dataset.live;
      if (action === "pick-file") $("#live-file")?.click();
      else if (action === "send-input") send();
      else if (action === "dismiss-error") emit("dismiss-error");
      else if (action.startsWith("delete:")) {
        if (window.confirm("Delete this message?")) emit(action);
      } else emit(action);
      return;
    }
    if (e.target.matches("[data-live-close]")) emit("close");
    const copy = e.target.closest("[data-copy]");
    if (copy) {
      e.preventDefault();
      navigator.clipboard?.writeText(copy.dataset.copy).catch(() => {});
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
      emit(kind === "auth" ? "auth" : `submit:${kind}`);
    }
    if (e.target.matches("[data-gif-search]")) {
      e.preventDefault();
      emit(`gif-search:${$("#gif-query").value}`);
    }
  });
  document.addEventListener("input", (e) => {
    if (e.target.id === "live-input") {
      const c = $("#live-messages")?.dataset.channel;
      if (c) remember(c, e.target.value);
      clearTimeout(typingTimer);
      typingTimer = setTimeout(() => emit("typing"), 200);
    }
    if (e.target.matches("[data-emoji-query]")) {
      clearTimeout(typingTimer);
      const query = e.target.value;
      typingTimer = setTimeout(() => emit(`emoji-query:${query}`), 120);
    }
  });
  document.addEventListener("change", (e) => {
    if (e.target.id === "live-file") emit("upload");
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
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      if (!$("#live-viewer")?.hidden) closeViewer();
      else emit("close");
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
    const modal = $(".live-modal");
    if (modal && e.key === "Tab") {
      const items = [
        ...modal.querySelectorAll("button,input,textarea,select,a[href]"),
      ].filter((e) => e.getClientRects().length && !e.disabled);
      const first = items[0],
        last = items.at(-1);
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  });
  document.addEventListener(
    "scroll",
    (e) => {
      const list = e.target;
      if (list.id !== "live-messages") return;
      const rank = Math.floor(list.scrollTop / 88),
        start = Number(list.dataset.windowStart),
        end = Number(list.dataset.windowEnd),
        total = Number(list.dataset.total);
      if (
        total > 50 &&
        (rank < start || rank + Math.ceil(list.clientHeight / 88) > end + 5) &&
        rank < total - 5
      ) {
        clearTimeout(seekTimer);
        seekTimer = setTimeout(() => emit(`seek:${rank}`), 120);
      }
      if (list.scrollHeight - list.scrollTop - list.clientHeight < 80)
        emit("read");
    },
    true,
  );
  const focus = () => emit(`focus:${!document.hidden && document.hasFocus()}`);
  window.addEventListener("online", () => emit("recover"));
  window.addEventListener("focus", focus);
  window.addEventListener("blur", focus);
  document.addEventListener("visibilitychange", focus);
  window.addEventListener("pagehide", () => {
    media?.action("leave");
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
  let touch;
  document.addEventListener("pointerdown", (e) => {
    if (
      e.pointerType === "touch" &&
      innerWidth < 768 &&
      !e.target.closest("input,textarea,[role=dialog]")
    )
      touch = { x: e.clientX, y: e.clientY };
  });
  document.addEventListener("pointerup", (e) => {
    if (touch) {
      const dx = e.clientX - touch.x,
        dy = e.clientY - touch.y;
      if (Math.abs(dx) > 80 && Math.abs(dx) > Math.abs(dy))
        emit(`panel:${dx > 0 ? "left" : "right"}`);
      touch = null;
    }
  });
  if ("serviceWorker" in navigator) {
    const register = () =>
      navigator.serviceWorker
        .register("/sw.js", { scope: "/" })
        .catch(() => {});
    if ("requestIdleCallback" in window)
      requestIdleCallback(register, { timeout: 2000 });
    else setTimeout(register, 0);
  }
}
let viewerScale = 1,
  viewerX = 0,
  viewerY = 0,
  drag;
function transform() {
  const img = $("#live-viewer img");
  if (img)
    img.style.transform = `translate(${viewerX}px,${viewerY}px) scale(${viewerScale})`;
}
function closeViewer() {
  const v = $("#live-viewer");
  if (v) {
    v.replaceChildren();
    v.hidden = true;
  }
}
function openViewer(target) {
  const viewer = $("#live-viewer");
  if (!viewer) return;
  viewerScale = 1;
  viewerX = viewerY = 0;
  viewer.replaceChildren();
  viewer.hidden = false;
  viewer.dataset.channel = $("#live-messages")?.dataset.channel ?? "";
  const img = new Image();
  img.alt = target.dataset.filename;
  const original = target.dataset.viewImage;
  const thumb = target.querySelector("img");
  if (thumb) img.src = thumb.src;
  const full = new Image();
  full.onload = () => {
    if (!viewer.hidden) img.src = full.src;
  };
  full.src = original;
  const toolbar = document.createElement("div");
  toolbar.className = "live-viewer-toolbar";
  for (const [label, fn] of [
    [
      "Zoom In",
      () => {
        viewerScale = Math.min(8, viewerScale * 1.25);
        transform();
      },
    ],
    [
      "Zoom Out",
      () => {
        viewerScale = Math.max(0.1, viewerScale / 1.25);
        transform();
      },
    ],
    [
      "Reset",
      () => {
        viewerScale = 1;
        viewerX = viewerY = 0;
        transform();
      },
    ],
    ["Close", closeViewer],
  ]) {
    const b = document.createElement("button");
    b.className = "button secondary";
    b.textContent = label;
    b.onclick = fn;
    toolbar.append(b);
  }
  const link = document.createElement("a");
  link.href = original;
  link.target = "_blank";
  link.rel = "noopener";
  link.textContent = `${target.dataset.filename} · Open original`;
  const footer = document.createElement("footer");
  footer.append(link);
  viewer.append(toolbar, img, footer);
  viewer.onclick = (e) => {
    if (e.target === viewer) closeViewer();
  };
  viewer.onwheel = (e) => {
    e.preventDefault();
    const old = viewerScale;
    viewerScale = Math.max(
      0.1,
      Math.min(8, viewerScale * (e.deltaY < 0 ? 1.2 : 1 / 1.2)),
    );
    viewerX =
      (viewerX - (e.clientX - innerWidth / 2)) * (viewerScale / old) +
      (e.clientX - innerWidth / 2);
    viewerY =
      (viewerY - (e.clientY - innerHeight / 2)) * (viewerScale / old) +
      (e.clientY - innerHeight / 2);
    transform();
  };
  img.ondblclick = () => {
    viewerScale = viewerScale === 1 ? 2 : 1;
    transform();
  };
  img.onpointerdown = (e) => {
    drag = { x: e.clientX - viewerX, y: e.clientY - viewerY };
    img.setPointerCapture(e.pointerId);
  };
  img.onpointermove = (e) => {
    if (drag) {
      viewerX = e.clientX - drag.x;
      viewerY = e.clientY - drag.y;
      transform();
    }
  };
  img.onpointerup = () => (drag = null);
}
