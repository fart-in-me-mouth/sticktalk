# Backlog: queued owner requirements (the lead assigns these to agents)

## B1. Image viewer (client): owner requirement
- **Opening and layout**
  - Clicking any image attachment, GIF or embed image opens a full-window viewer.
  - The backdrop is #000 at 85%. The image is centered and fit to the window with a 48 px margin.
  - Below the image: the filename, size and "Open original" link, all in 12 px muted text.
- **Loading:** fetch the **original only now**, never before. Show the thumbnail scaled up and blurred immediately, then swap in the original when it arrives, with a small static spinner and no animation loop.
- **Zoom and pan**
  - Mouse wheel zooms around the cursor, from 10% to 800%.
  - Double-click toggles between fit and 100%.
  - Dragging pans while zoomed.
  - `+`, `-` and `0` keys zoom in, zoom out and reset.
- **Toolbar** (top-right, bg.elevated, radius 8, 36 px icon buttons with tooltips)
  - Zoom in/out
  - Forward to another channel or DM
  - Save, which uses the native Save dialog
  - Open in browser
  - More menu: Copy Image (to the clipboard), Copy Link
- **Navigation:** ← and → move between images in the same message, then the channel. A counter shows "2 / 5".
- **Closing:** Esc or a click outside the image.
- **Memory:** decode the original off the UI thread. Downscale to at most 2× the monitor resolution for display. Release the memory when the viewer closes.

## B2. Unlimited file uploads (server + client): owner requirement
- **No per-file size limit by default.** `--max-upload-mb 0` means unlimited, and that is the new default.
- **Uploads**
  - Resumable chunked upload, tus-style or a simple equivalent: create the upload, PUT chunks of 8–16 MB with offsets, resume after a disconnect or app restart, finish to commit.
  - The server streams chunks straight to disk and never buffers a whole file in RAM.
  - A SHA-256 integrity check on finish.
- **Downloads:** HTTP Range requests, so downloads can resume and video can seek.
- **Disk safety**, because the real limit is the VM disk (about 150 GB usable on Oracle Always Free):
  - The server refuses new uploads when free disk would drop below `--min-free-disk-gb` (default 10). The error says "Server storage is full."
  - Abandoned partial uploads are garbage-collected after 24 h.
  - Optional per-user quota flag, off by default.
  - Disk usage is shown in Server Settings for the owner.
- **Client**
  - The upload card shows a progress bar, speed, ETA and a cancel button. Uploads continue in the background while the user switches channels.
  - Big videos (mp4/webm) play inline with a simple player: play/pause, seek and volume. Decoding uses hardware where possible, and nothing loads until the user clicks play.
- Security: keep all the existing upload hardening from `crates/server/AUDIT.md`.

## B3. Notifications and tray (client): before beta
- **Desktop notifications** for DMs and @mentions, using Windows toast notifications.
- **Sounds:** message, mention, and later call join and leave.
- **Taskbar:** an unread badge overlay on the taskbar icon, and the taskbar flashes on a mention.
- **Tray:** minimize to the system tray, and "Start StickTalk when Windows starts" (off by default).
- **Mute:** per-server and per-channel mute and notification settings, with options all messages / only @mentions / nothing.
- No notifications for the channel the user is currently looking at while the window is focused.

## B4. Installer and auto-update: before beta
- **Installer:** a single small installer (NSIS or WiX MSI) that installs per-user, with no admin needed.
- **Updates**
  - The client checks `GET /api/update/latest` on launch and every 6 h.
  - Updates are verified with an **ed25519 signature** before they're installed. The public key is embedded in the client.
  - Updates download in the background, and the app restarts into the new version on the user's confirm.
- The owner gets a one-command release script that builds, signs, and uploads the new version to the server.

## B5. Account recovery: before beta
- A server admin CLI: `sticktalk-server admin reset-password <username>`, which prints a one-time reset code. It can also list users and disable a user.

## B6. End-to-end test with the real server: before beta
- Run the real client against the real server: every Phase 1 flow, Google sign-in with a mock Google, GIFs with a mock provider, uploads, and reconnects.

## B7. GIF picker (client)
Per `docs/GIFS.md`, once the server side lands.

## B8. Web app + phones (Rust → WebAssembly, installable PWA): owner priority
Goal: StickTalk runs in any browser at `https://<server>/`, and can be **installed to the phone home screen** (iOS Safari and Android Chrome) as a PWA. It must be convenient enough to replace the Discord mobile app for this group.

### Decision: why the web/phone UI uses Leptos, not iced compiled to wasm
iced in the browser renders to a canvas. On phones that breaks:
- the virtual keyboard, autocorrect, IME and text selection
- native scrolling momentum
- accessibility and pinch-zoom

So:
- **Desktop app** stays native **iced**. It is the lightest option on Windows.
- **Web + phones** use **Leptos**, a Rust → wasm framework that renders real DOM elements: real inputs, native scrolling, PWA support. Both clients share one Rust crate, **`client-core`**, which contains:
  - state and reducers
  - protocol and network logic over a transport trait
  - the markdown parser
  - the unread, permissions and formatting logic

  Each UI layer is only views.
- Design tokens come from `docs/UI_SPEC.md`. Generate CSS custom properties from the same token table, so desktop and web stay visually identical.

### Requirements
- **Layout**
  - **Responsive.** Desktop-width browsers get the same layout as the desktop app.
  - **Phones (< 768 px)** get a Discord-mobile-style layout:
    - Left panel: the server rail plus channel list. Swipe right to reveal it.
    - Center: the chat.
    - Right panel: the member list. Swipe left to reveal it.
    - Bottom tab bar: Home / Notifications / You.
  - Safe-area insets for notches.
  - The composer sticks above the keyboard, using the `visualViewport` API.
- **PWA**
  - manifest, icons, splash screen
  - a service worker that caches the app shell, so it opens instantly and offline shows a "Reconnecting…" state
  - **Web Push** notifications for DMs and mentions, using VAPID keys generated on the server. These work on iOS 16.4+ when the app is installed to the home screen, and on Android.
- **Media on phones**
  - Voice and video use the browser's WebRTC to the SFU.
  - Screen share works where the browser supports `getDisplayMedia`, which means desktop browsers. Phones can **watch** streams but not share.
- **Performance**
  - Brotli-compressed wasm under 1.5 MB. Total first load under 2.5 MB including fonts.
  - Interactive in under 2 s on a mid-range phone over 4G.
  - Virtualized message list.
  - Images get `loading=lazy` plus server thumbnails, with `srcset` for DPR.
  - 60 fps scrolling on a mid-range phone.
- **Server**
  - Serves the web build, same origin as the API.
  - Browser sessions use `Secure`, `HttpOnly`, `SameSite=Strict` cookies plus a CSRF token.
  - Web-redirect variant of Google OAuth.
  - Web Push subscription endpoints.
- **Desktop-only:** remote control, tray, and hardware-encoded 1080p60+ screen share. The web uses the browser's encoder.

### Order
1. Extract `client-core` from `crates/client`. The desktop must still pass all its tests.
2. Leptos web client: desktop-width layout.
3. Phone layout plus PWA plus Web Push.

## B9. Hosting decision: Raspberry Pi 4 (2 GB+) over Tailscale (primary). Oracle is the fallback/backup.
- **Network**
  - The server runs on a friend's Pi 4. Every user reaches it over **Tailscale**: the Pi owner shares the node with each friend, and friends use their own free Tailscale accounts.
  - No port forwarding.
  - HTTPS via `tailscale cert` for the `*.ts.net` MagicDNS name, through Caddy or rustls directly.
- **Build**
  - **Cross-compile from Windows.** Do not build on the Pi: fat LTO plus 2 GB RAM is slow and can run out of memory.
  - Use `cargo zigbuild --target aarch64-unknown-linux-gnu` (or `cross`).
  - Add a `deploy/pi/` kit: a build script, scp/rsync deploy over Tailscale SSH, a systemd unit, and a setup guide for 64-bit Raspberry Pi OS Lite. **Data dir on a USB SSD, not the SD card.** Log to journald with size limits.
- **Voice/WebRTC over Tailscale**
  - The SFU binds and advertises ICE host candidates on the Pi's Tailscale IP (100.x).
  - Keep RTP packets at most 1200 bytes, because the WireGuard MTU is 1280.
  - The UDP port range is configurable.
  - Test with relayed DERP paths too (higher latency) and surface the "relayed connection" status in the call UI.
- **Pi 4 limits**
  - The SFU only forwards packets, so CPU is fine.
  - The real limit is the home **upload** bandwidth: about 8 Mbps per viewer at 1080p60.
  - Show the actual per-viewer bitrate in the stream stats overlay.
  - The server should cap the total outgoing video bitrate with a configurable flag. When the cap is hit, it asks senders to lower their bitrate (via REMB/TWCC) instead of letting all streams stutter.
- **Backups:** nightly SQLite `.backup` plus rsync of uploads to a second location: another friend's PC over Tailscale, or Oracle object storage.
- **Client:** the default server address is a build-time constant, set to the Pi's MagicDNS URL.
