# Browser / phone server deployment

The server can host a prebuilt Leptos PWA on its existing HTTPS origin. The web
client and service worker are built separately; this change does not build them.

1. Copy the web distribution to a read-only directory, e.g. `/opt/sticktalk/web`.
   Include `index.html`, manifest, icons and service worker. Build content-hashed
   JS/CSS/wasm/font names and retain original files beside `.br` / `.gz` variants.
   The server streams and negotiates those variants without runtime compression.
2. Set these in the service's protected EnvironmentFile (not in git):

   ```ini
   STICKTALK_WEB_ROOT=/opt/sticktalk/web
   STICKTALK_PUBLIC_URL=https://your-node.your-tailnet.ts.net
   STICKTALK_GIF_MEDIA_ORIGINS=https://media.klipy.com
   STICKTALK_VAPID_SUBJECT=mailto:operator@example.com
   # Optional: separate Google Web application credentials, NOT desktop credentials
   STICKTALK_GOOGLE_WEB_CLIENT_ID=your-web-client.apps.googleusercontent.com
   STICKTALK_GOOGLE_WEB_CLIENT_SECRET=your-private-web-client-secret
   ```

   Replace the GIF media origins with the exact CDN origins supplied by your
   configured provider. No wildcards or paths. Use same-origin GIF metadata proxy
   mode (with provider approval); strict CSP does not grant direct provider API
   connections. Do not enable COEP for the current single-threaded wasm build.
3. Google Cloud: add a **Web application** OAuth client and register exactly
   `https://your-node.your-tailnet.ts.net/api/v1/browser/google/callback` as an
   authorized redirect URI. Publish the consent screen for normal use. Desktop
   Google client configuration remains separate. Never ship the web client secret
   to a browser bundle or expose it via metadata. See the browser protocol section
   for PKCE, signup and cookie contracts.
4. Keep Caddy/Tailscale TLS in front of the server; proxy `/` and `/api/*` together
   to the same loopback HTTP listener. No separate static vhost/CORS is necessary.
   Existing Caddyfiles already proxy all paths. Do not override the server CSP or
   Cookie headers. Keep API responses and OAuth callback URLs out of access logs.
5. Data directory: back up SQLite **and** `vapid.pk8`. The latter is generated on
   first startup, is a private ES256 key, and must retain owner-only permissions
   (Unix mode 0600; owner-only data directory/ACLs on Windows). Lost/rotated keys
   require browsers to unsubscribe/re-subscribe using the new `/push/key`.
6. Push egress uses only HTTPS to FCM, Firefox Push and Apple Web Push by default.
   Add a genuinely required trusted provider via `STICKTALK_PUSH_ORIGINS`
   (comma-separated replacement allowlist). Do not allow arbitrary/internal
   origins. Set a useful VAPID contact URI. Delivery is best-effort, queue-bounded,
   coalesced and rate-limited; it is not an offline message queue.
7. Wire the notification-settings implementation using
   `App::set_push_policy(Arc<dyn push::NotificationPolicy>)` before serving.
   Hook failures deny notifications. The default honors DND, read-state, block,
   focus and access revocation, but custom per-server/channel mute tables require
   that hook. Have clients send `ClientMsg::Focus` on visibility/focus changes.

Browser acceptance still needed: install on iOS/Android, grant notification
permission through a user gesture, subscribe with `/push/key`, close/unfocus the
app, send a DM/mention, and verify encrypted delivery and notification-click
navigation using the server-generated payload `url` (which preserves 64-bit IDs). iOS requires an installed home-screen PWA. Never use real system
notifications or browser authorization windows in automated tests.

Local automated tests use loopback-only HTTP/UDP/SFU listeners and mock Google
JWKS/token and push services. `STICKTALK_MOCK_GOOGLE_TOKEN_URL` and
`STICKTALK_MOCK_PUSH_ORIGIN` are explicit loopback-only test overrides; never use
these on a deployed service. Dev/test servers must bind 127.0.0.1/::1 (including
`--voice-bind-ip` and `--voice-advertised-ip`), never wildcard/LAN/Tailscale.
No administrator install, firewall-rule changes or UAC actions are needed.

### Bundled web-shell CSP compatibility

The Trunk post-build hook externalizes its module bootstrap, boot styles and
UI_SPEC-derived tokens into content-hashed same-origin assets. Inline scripts
and style elements remain blocked. Only `style-src-attr` allows inline values,
for validated avatar/role colors and virtualized layout geometry. Run the web
`test:server` smoke test against the native `--web-root` server, not just the
fixture server; it asserts zero application CSP violations and verifies that
injected inline script/style elements are blocked.
