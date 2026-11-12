# Sign in with Google: owner requirement (queued after the current Phase 1 agents finish)

## Flow (OAuth 2.0 for installed apps, RFC 8252)
1. The client shows a "Continue with Google" button on the login and register screens. It is a secondary button with the official multicolor "G" mark, following Google's branding guidelines.
2. The client starts a loopback listener on `127.0.0.1:<random port>`, then opens the **system browser** to Google's authorization endpoint with:
   - `response_type=code`
   - `scope=openid email profile`
   - **PKCE (S256)**
   - `state`
   - `redirect_uri=http://127.0.0.1:<port>`
   No embedded webview, because Google blocks those.
3. The browser redirects back to the loopback listener. The client checks `state`, shows a small "You can close this tab" page, and brings the StickTalk window to the front.
4. The client exchanges the code plus the PKCE verifier at Google's token endpoint and gets an `id_token`. (A desktop-type OAuth client's "secret" is not confidential; ship it in the client config, never in the git repo.)
5. The client sends `POST /api/auth/google { id_token }` to the StickTalk server.
6. The server verifies the `id_token`:
   - The RS256 signature against Google's JWKS. Cache the keys and respect the `Cache-Control` max-age.
   - `iss` is `https://accounts.google.com` or `accounts.google.com`.
   - `aud` equals the configured client ID.
   - `exp` and `iat` are valid, with ±60 s clock skew allowed.
   - `email_verified` is true.
7. The server finds or creates the account:
   - Look up the identity by Google `sub`. Never look it up by email alone.
   - Store it in an `oauth_identities(provider, subject, user_id)` table.
8. The server responds:
   - **Existing user**: the normal session token, the same as password login.
   - **New user**: a short-lived (10 min) signup ticket. The client shows a "Pick a username" screen, then calls `POST /api/auth/google/complete { ticket, username }`, which creates the user and returns a session.
9. Account settings let a password user link or unlink Google. A Google-only user can set a password. Unlinking is blocked if it would leave no way to log in.

## Config
- Server flag/env `--google-client-id`. If it is unset, the endpoint returns `404` / `FeatureDisabled`, and the `/api/meta` (or equivalent) capability flag tells the client to hide the button.
- The client gets the client ID from the server's capability endpoint, so one build works with any server.

## Proto / tests
- Additive proto changes only, documented in `PROTOCOL.md`.
- Server tests use a locally generated RSA key and a mocked JWKS to cover:
  - valid token
  - wrong `aud`
  - expired token
  - unverified email
  - replayed signup ticket
  - username collision
  - link/unlink rules
- Client test: PKCE and `state` generation and validation.

## Owner setup (free, about 5 minutes; walk them through it in deploy/README.md)
1. In Google Cloud Console, create a project.
2. Set up the OAuth consent screen: External, app name "StickTalk", scopes `openid`, `email`, `profile` only.
3. **Publish** the consent screen. Basic scopes need no Google review. Leaving it in "Testing" limits sign-in to 100 manually added test users.
4. Create credentials: an OAuth client ID of type **Desktop app**.
5. Put the client ID and secret on the server as env vars.
