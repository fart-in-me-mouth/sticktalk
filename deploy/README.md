# StickTalk: Oracle Always Free, ARM64 Ubuntu 24.04

One native Rust process, SQLite/files on local disk, Caddy for HTTPS. Commands
below run in the VM's SSH terminal unless labelled **Windows**. Keep SSH open
while changing networking. Keep off-VM backups and install OS updates.

## 1. Create the VM, staying within the free tier

1. Create an Oracle Cloud account. In your **home region**, open Compute →
   Instances → Create instance. Choose an **Always Free eligible Ubuntu 24.04
   ARM64** image and **VM.Standard.A1.Flex**, not an AMD/paid shape.
2. Choose **1 OCPU, 6 GB memory**, default ~50 GB boot volume, public subnet and
   public IPv4. November 2026 documentation lists **2 OCPUs / 12 GB total** A1
   allowance for Always Free tenancies, not the 4/24 in old tutorials. Combined
   boot/block storage allowance is 200 GB in the home region. Verify your own
   current limits and final estimate before clicking Create.
3. Save the SSH private key privately; note the public IP. Avoid paid images,
   load balancers, additional disks/backups and paid monitoring. Set a billing
   budget alert (an alert does **not** cap charges). Don't upgrade to a paid
   account just to obtain capacity.
4. If A1 is out of capacity, try another availability domain in your home region
   or wait; don't substitute a paid shape. Idle free VMs can be reclaimed, so
   keep backups elsewhere. [Oracle's current free-resource policy](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm).

**Windows PowerShell**, replacing key path and IP:

```powershell
ssh -i "$HOME\Downloads\ssh-key.key" ubuntu@YOUR_PUBLIC_IP
```

## 2. Open BOTH firewalls; leave 8080 private

**Oracle console:** instance → attached VNIC → subnet → security lists → the
subnet's security list → Add Ingress Rules. Create two **stateful TCP** rules:
source `0.0.0.0/0`, source ports all, destination **80** and **443**. Keep SSH 22
restricted to your public IP `/32` where possible. Attached NSGs also need
compatible rules. Keep outbound Internet access for DNS, updates and certificates.
Do not expose TCP 8080; there is no SQLite network port to open.

**Ubuntu:** Oracle images can block inbound traffic in iptables even when UFW
says inactive. Insert rules **before** the final REJECT, not after it:

```bash
sudo apt-get update
sudo apt-get install -y iptables-persistent
sudo iptables -C INPUT -p tcp --dport 80 -j ACCEPT 2>/dev/null || sudo iptables -I INPUT 1 -p tcp --dport 80 -j ACCEPT
sudo iptables -C INPUT -p tcp --dport 443 -j ACCEPT 2>/dev/null || sudo iptables -I INPUT 1 -p tcp --dport 443 -j ACCEPT
sudo netfilter-persistent save
sudo iptables -L INPUT -n --line-numbers
```

Do **not** flush INPUT/OUTPUT or delete Oracle infrastructure rules. Don't mix
UFW with this setup unless you understand both chains. This guide uses IPv4:
don't publish AAAA records. IPv6 needs its own VCN routes/rules and `ip6tables`.
After setup reboot and retest HTTPS to check persistence.
[Oracle explains Ubuntu's firewall and REJECT precedence](https://blogs.oracle.com/developers/enabling-network-traffic-to-ubuntu-images-in-oracle-cloud-infrastructure).

## 3. Copy sources and build natively

Install Git, then clone **your own actual repository URL**:

```bash
sudo apt-get install -y git
git clone YOUR_REPOSITORY_URL sticktalk
cd sticktalk
bash deploy/install.sh
```

For private sources use your existing Git authentication; don't put tokens in
URLs or chat. Alternatively copy the source with `scp` from Windows (exclude
build artifacts, `.git`, databases and uploads). Runtime data must not be synced.

If the project is only on your Windows computer, open PowerShell **in the project
folder** and use this instead of Git (replace the key and public IP):

```powershell
tar.exe -czf "$env:TEMP\sticktalk-src.tgz" --exclude=.git --exclude=target --exclude=sticktalk-data --exclude='*.sqlite3*' .
scp -i "$HOME\Downloads\ssh-key.key" "$env:TEMP\sticktalk-src.tgz" ubuntu@YOUR_PUBLIC_IP:sticktalk-src.tgz
```

Then in the VM's SSH terminal:

```bash
mkdir -p "$HOME/sticktalk"
tar -xzf "$HOME/sticktalk-src.tgz" -C "$HOME/sticktalk"
cd "$HOME/sticktalk"
bash deploy/install.sh
```

Read `deploy/install.sh` first. It installs native build tools, Rust stable
(minimal profile), SQLite tools, an unprivileged service account, and Caddy.
Re-running preserves data, `/etc/sticktalk.env` and the existing Caddy config;
it builds a new server without restarting the old one. Initial compilation may
take minutes. `uname -m` should say `aarch64`; no cross compiler or Docker needed.

Manual build after installing Rust and build tools:

```bash
. "$HOME/.cargo/env"
export CARGO_TARGET_DIR="$HOME/.cache/sticktalk-server-target"
cargo build --locked --release -p sticktalk-server
```

The target-dir override is essential: the repository's default is a Windows
path. Don't compile the GUI on this VM. Keep persistent data on local disk.

## 4. Free DuckDNS hostname

Visit [DuckDNS](https://www.duckdns.org/), sign in, create a unique subdomain
(e.g. `my-sticktalk`), and set its IPv4 to your VM's public IP. The result is
`my-sticktalk.duckdns.org`. Allow DNS time to propagate. Don't use this example
or the repository's placeholder literally.

Update the dashboard whenever the VM's IP changes. Optional automatic updates:
follow DuckDNS's [official Linux cron instructions](https://www.duckdns.org/install.jsp?tab=linux-cron).
Keep the DuckDNS token in a private file (`chmod 600`), not Git, chat, shell
history, screenshots or app logs. Verify Oracle pricing before reserving IPs.

## 5. Start the service and enable HTTPS

```bash
sudo nano /etc/sticktalk.env
# Set: STICKTALK_PUBLIC_URL=https://my-sticktalk.duckdns.org
sudo systemctl restart sticktalk
sudo systemctl status sticktalk --no-pager
curl http://127.0.0.1:8080/api/v1/health

nano deploy/Caddyfile
# Replace YOUR-NAME.duckdns.org, save, then:
sudo install -m 0644 deploy/Caddyfile /etc/caddy/Caddyfile
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl enable --now caddy
sudo systemctl reload caddy
```

Caddy obtains/renews HTTPS certificates and proxies WebSocket upgrades
automatically. DNS and both firewalls must work first. The app stays on
`127.0.0.1:8080`: never expose public plaintext bearer tokens. It runs as
`sticktalk` with hardened systemd settings, private files and data in
`/var/lib/sticktalk`. [Caddy installation](https://caddyserver.com/docs/install),
[reverse-proxy behavior](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy).

**Windows**, test from outside the VM:

```powershell
curl.exe https://my-sticktalk.duckdns.org/api/v1/health
```

Enter `https://my-sticktalk.duckdns.org` in the native client. Do not bypass TLS
verification. For failures check `getent ahostsv4 HOSTNAME`, both firewalls,
`sudo journalctl -u caddy -n 50 --no-pager`, and
`sudo journalctl -u sticktalk -n 50 --no-pager`. Don't share private diagnostic
data. Native clients do not need CORS.

### Configuration

Flags override env: `--bind` / `STICKTALK_BIND` (127.0.0.1:8080), `--data-dir` /
`STICKTALK_DATA_DIR` (./sticktalk-data), `--public-url` / `STICKTALK_PUBLIC_URL`
(http://localhost:8080), `--max-upload-mb` / `STICKTALK_MAX_UPLOAD_MB` (25, may
lower but not raise), `--worker-id` / `STICKTALK_WORKER_ID` (0). Only one process
may use a data directory. `--trusted-proxy` / `STICKTALK_TRUSTED_PROXY`
defaults to 127.0.0.1 (Caddy); only that peer may supply client IP headers. No network/OneDrive storage. Registration is open:
share your hostname only with intended users.

The global file admission budget is 10 GiB (`--max-total-upload-mb` /
`STICKTALK_MAX_TOTAL_UPLOAD_MB`), including pending files, thumbnails, and
in-flight reservations. Set it below your available volume capacity, leaving
headroom for SQLite/WAL, OS updates, and backups. This is not a filesystem quota:
monitor free space and restrict network access if open registration is unwanted.
HTTP work is capped at 128 concurrent responses, with 16 upload slots and
15-second request / 120-second upload deadlines. Caddy also limits headers and
body time. The service has a 768-MiB soft / 1-GiB hard memory limit; monitor and
remeasure on ARM before increasing it. A hard-limit kill requires client resync.

## Optional: turn on “Continue with Google” (about five minutes)

You can skip this section: username/password accounts still work. Google sign-in
uses basic identity information only, not Gmail, Drive, contacts, or paid APIs.
Keep the server's HTTPS working before enabling it.

1. Open [Google Cloud Console](https://console.cloud.google.com/) in your browser
   and sign in with the Google account you want to own StickTalk's setup. Open
   the project selector at the top, choose **New project**, name it **StickTalk**,
   then create and select it. Do not enable unrelated paid services.
2. Find **Google Auth Platform** (older menus call it **APIs & Services → OAuth
   consent screen**). Start setup. Use **StickTalk** for the app name, your email
   for support/contact, and **External** for the audience so other people can
   sign in. Fill in truthful app/contact details that Google requests.
3. In **Data Access / Scopes**, select **only** `openid`, `email`, and `profile`
   (the last two may appear as `.../auth/userinfo.email` and
   `.../auth/userinfo.profile`). Do not add Gmail, Drive, or any sensitive scope.
   Save. These basic sign-in scopes need no sensitive-scope Google review;
   Google may still request branding/domain or policy details for production.
4. **Important: PUBLISH the consent screen.** Open **Audience**, find
   **Publishing status**, click **Publish app**, and confirm. Check that it
   says **In production**, not **Testing**. Testing normally uses a manually
   managed test-user list (up to 100). Google's current policy exempts requests
   using only these basic identity scopes from that restriction; nevertheless,
   **publish StickTalk for production**, rather than relying on a testing exception.
5. Open **Clients → Create client** (older menu: **Credentials → Create
   credentials → OAuth client ID**). For application type choose **Desktop
   app** — **NOT Web application**. Name it **StickTalk desktop**, then create
   it. The native app uses your system browser and a temporary local return
   address; do not add your DuckDNS hostname as a web redirect.
6. Copy the **Client ID** (it ends in `.apps.googleusercontent.com`). In your
   VM's SSH terminal open the private server settings file:

   ```bash
   sudo nano /etc/sticktalk.env
   ```

   Add a line using your own client ID, not this placeholder:

   ```text
   STICKTALK_GOOGLE_CLIENT_ID=YOUR_DESKTOP_CLIENT_ID.apps.googleusercontent.com
   ```

   Save, exit the editor, then run:

   ```bash
   sudo chmod 0600 /etc/sticktalk.env
   sudo systemctl restart sticktalk
   curl http://127.0.0.1:8080/api/v1/meta
   ```

   The reply should contain your `google_client_id`, not `null`. You can also
   use the flag `--google-client-id` when starting the executable manually.
7. Google also displays a **client secret** or offers a credentials JSON
   download. Keep that download **outside the source checkout**, in a private
   location on your server if you need to retain it for client distribution.
   Never paste it into Git, chat, screenshots, `/meta`, or this README. The
   **server verifier does not use a client secret**: the desktop client handles
   the code exchange and needs its own locally supplied client configuration
   for Google's desktop secret. Merely setting a secret on the server does not
   configure desktop clients. Follow the separately implemented client's setup
   instructions; do not invent a public secret-download endpoint. Desktop
   secrets are not confidential credentials like a web-server secret, so PKCE
   and state remain essential. The public client ID comes from `/api/v1/meta`.
8. Reopen StickTalk, choose your HTTPS server, and try **Continue with Google**.
   A first sign-in asks you to choose a StickTalk username. Existing password
   users should sign in normally first and **link** Google in account settings;
   matching email addresses do not join accounts. Before unlinking Google from
   a Google-only account, set a password in account settings.

If the button is hidden, check that `/api/v1/meta` has the client ID and restart
both service and client. For `access_denied`, recheck **Publish / In production**
and the selected Google account. A wrong client type or mismatched client ID
can prevent sign-in; recreate a **Desktop app** credential rather than disabling
PKCE/TLS checks. Removing `STICKTALK_GOOGLE_CLIENT_ID` and restarting disables
new Google sign-ins, but doesn't revoke existing StickTalk sessions. Let
Google-only users set passwords before disabling it to avoid locking them out.
Never share ID tokens or signup tickets when troubleshooting.

Google's authoritative references: [desktop client and PKCE setup](https://developers.google.com/identity/protocols/oauth2/native-app),
[production and consent policies](https://developers.google.com/identity/protocols/oauth2/production-readiness/policy-compliance),
[OAuth publishing-status help](https://support.google.com/cloud/answer/15549945).

## Optional GIFs: provider decision and setup (checked 2026-11-12)

**Recommendation: KLIPY, client-direct first; approved metadata proxy later.**
No candidate currently grants the original server-proxy/caching design by
default. Nothing is enabled by this change. Do not treat a production API key
as permission to proxy or cache. The implementation includes a KLIPY adapter
and a provider-neutral server contract; no live provider was called in tests.

### Provider comparison: permission, price and waiting time

| Provider | Availability / free access | Client-direct fallback | What the owner must do for proxying; timing |
|---|---|---|---|
| **Tenor** | No new integrations since Jan 13, 2026; third-party API retired June 30, 2026. No usable new free-key path. | Not a supported fallback after retirement. | There is no published application path to restore third-party service, production access or proxy permission. Waiting time is **not applicable**; do not reuse leaked/legacy keys. [Google's retirement notice](https://support.google.com/tenor/answer/10455265?hl=en). |
| **GIPHY** | Free self-service beta key: 100 calls/hour. Higher-volume production access requires an application and pricing discussion; **not guaranteed free**. | Explicitly required for standard API/media use. Obtain an app/platform key in its developer dashboard; show required GIPHY attribution and follow ranking, tracking and URL-preservation requirements. A limited beta key may suffice for a small private app. | Standard rules prohibit API/media proxying and unapproved media/URL caching. Ask `api@giphy.com` / `developers@giphy.com` for written permission for the exact architecture; media-caching exceptions are separately approved. A production upgrade alone does not grant this. Key creation is self-service (allow a few minutes for setup, **not an SLA**); production/exception review has **no published turnaround**, acceptance or free-price guarantee. [API rules and key policy](https://developers.giphy.com/docs/api/), [FAQ/pricing](https://developers.giphy.com/faq), [design/attribution](https://developers.giphy.com/docs/). |
| **KLIPY (implemented)** | Available, free API advertised by KLIPY; test key 100 calls/hour, production access requested in Partner Panel. Ads are optional. | Standard requirements explicitly call for requests from the end-user app/browser and direct media delivery. Create a platform key, implement attribution/content rules, and request production access when ready. Use `Search KLIPY` in the search input and this app's `Powered by KLIPY` footer. | Email `developers@klipy.com` **before enabling proxy mode**, requesting approval for server-origin API calls, ten-minute in-memory metadata/URL caching and shared nonpersonalized results. Describe ID-only per-account favorites, unchanged direct media URLs and retained chat links. Do not request/implement a partner media cache here. Keys are self-service (minutes for setup); production and custom approval have **no published review SLA**. Free API marketing does **not** establish that a bespoke proxy agreement is free: get that confirmed in writing. [Current integration requirements](https://docs.klipy.com/migrate-from-tenor/register-share), [API terms](https://klipy.com/support/api-terms), [current key/production/ads guidance](https://klipy.com/developers), [KLIPY's free API announcement/docs](https://www.postman.com/klipy-api/klipy-api-services/overview). |

KLIPY is the recommendation because its API remains open to new developers,
it advertises free access, and the documented Tenor-compatible API supports
small video previews, categories and ID lookup. This is an engineering
recommendation, **not** a claim that custom proxy approval is assured. Use
direct mode if approval is unavailable. That deliberately relaxes GIFS.md's
IP-privacy requirement: KLIPY sees the user's IP for searches and media loads.
The client engineer must implement the advertised direct mode; setting the
server flags does not add that client code.

Neither standard provider policy authorizes the full persistent media/disk
cache proposed in GIFS.md. Keep media loading direct and uncached beyond what
the provider permits until explicit additional permission is obtained. Server
favorites persist **only opaque IDs**, never copied GIFs, titles or media URLs.
Clients resolve IDs when opening Favorites, so removed content can disappear
without copying a provider content database. Confirm any provider-specific
retention/attribution requirements before launch. These policies may change;
review the linked primary sources when applying, not old tutorials.

### Get a free KLIPY test key and enable direct mode

1. Visit the [KLIPY Partner Panel](https://partner.klipy.com/), create an account,
   open **API Keys**, and create a platform/application for StickTalk desktop.
   Supply accurate private-chat-app details; choose API access, not a paid
   unrelated product. Do not use a shared sandbox key from sample code.
2. Start with the free testing key, limited to **100 API calls/hour**. Disable
   optional ads for this integration in the app details and configure suitable
   content filters. Follow current branding/integration instructions, including
   `Search KLIPY` placeholder, visible attribution and any required share
   reporting. For live production access, submit the Partner Panel application
   and wait for approval; do not presume instant or guaranteed acceptance.
3. For the direct fallback, designate a **client-distributable** platform key.
   It will intentionally be public in `/api/meta`. Do not reuse any confidential
   server-only key or put a secret into this setting. In your VM terminal:

   ```bash
   sudo nano /etc/sticktalk.env
   ```

   Add your values (never the placeholder):

   ```text
   STICKTALK_GIF_PROVIDER=klipy
   STICKTALK_GIF_MODE=direct
   STICKTALK_GIF_PUBLIC_CLIENT_KEY=YOUR_CLIENT_DISTRIBUTABLE_PLATFORM_KEY
   ```

4. Save, then:

   ```bash
   sudo chmod 0600 /etc/sticktalk.env
   sudo systemctl restart sticktalk
   curl http://127.0.0.1:8080/api/meta
   ```

   Expect `gifs: {"mode":"direct","provider":"klipy","public_client_key":...}`.
   Searches/categories/ID lookups go **client → KLIPY**, not through StickTalk.
   StickTalk discovery endpoints return 404 `feature_disabled` in direct mode;
   favorites GET/PUT/DELETE remain available. The key is not a StickTalk bearer
   credential; never send StickTalk tokens to KLIPY. Never paste keys into Git,
   issue reports, screenshots or chat.

### Enable the proxy only after explicit written approval

Send the architectural description above to `developers@klipy.com`; ask them
to confirm permission, pricing, quota and any cache expiration/purge conditions.
This implementation has a fixed ten-minute metadata TTL and **no media cache**.
If their approval requires other behavior, do not enable it unchanged.
Then configure a separate server-only key in `/etc/sticktalk.env`:

```text
STICKTALK_GIF_PROVIDER=klipy
STICKTALK_GIF_MODE=proxy
STICKTALK_GIF_API_KEY=YOUR_SERVER_ONLY_KEY
STICKTALK_GIF_PROXY_APPROVED=true
```

Remove the direct public-key line unless you still need to retain it privately.
Restart and verify `/api/meta` returns exactly the proxy config
`{"mode":"proxy","provider":"klipy"}` without either key. The approval setting
is an operator attestation, not an automated check or provider-issued permission.
It is deliberately required **in addition to** provider and key. Equivalent
manual flags: `--gif-provider klipy --gif-mode proxy --gif-api-key ...
--gif-proxy-approved`; prefer the private env file so secrets aren't in shell
history/process arguments. Direct mode uses `--gif-public-client-key`, not
`--gif-api-key`. Environment settings mirror these flag names with the
`STICKTALK_` prefix and underscores.

Without an explicit provider and appropriate key, meta contains `"gifs":false`.
Proxy without approval is also disabled; direct is the default mode. Bad empty,
oversized or control-containing keys fail initialization. Removing the provider
setting and restarting disables discovery without deleting saved favorite IDs.
The public `/meta` endpoint itself is never a credential test: an invalid but
nonempty key produces provider errors when used. An enabled flag does not
prove a key is valid, a quota is sufficient, or an approval has been granted.

Proxy bounds: 24 results/page, eight categories; metadata TTL ten minutes,
128 cache entries / 8 MiB serialized metadata, one coalesced refresh at a time.
Every account has 30 picker/favorites requests/minute, burst ten across all
sessions. Upstream calls are conservatively capped at **100/hour globally**,
even with a production key; a cold category response uses up to nine calls
(category list plus eight nano-preview searches). Failures consume provider
quota. No GIF background polling, media downloads or rehosting. Favorites are
limited to 200/account. SQLite migration `0007_gif_favorites.sql` runs at startup;
ordinary database backups include them. Endpoint/DTO and direct normalization
details: [PROTOCOL.md](../docs/PROTOCOL.md#optional-gif-discovery-and-account-favorites-additive-v1).

## 6. Backups: SQLite `.backup` PLUS files

Copying only the live SQLite main file loses WAL contents. `.backup` produces
a coherent database, but its files must match. For a simple reliable backup,
stop the app briefly (leave Caddy running):

Prefer the checked-in helper, which uses private directories, checks SQLite,
and restores the service's prior running state even when copying fails:

```bash
sudo bash deploy/backup.sh
```

The following equivalent manual sequence needs an explicit restart if interrupted:

```bash
sudo systemctl stop sticktalk
sudo install -d -m 0700 /var/backups/sticktalk
stamp=$(date -u +%Y%m%dT%H%M%SZ)
sudo mkdir -m 0700 "/var/backups/sticktalk/$stamp"
sudo sqlite3 /var/lib/sticktalk/sticktalk.sqlite3 ".backup '/var/backups/sticktalk/$stamp/sticktalk.sqlite3'"
sudo cp -a /var/lib/sticktalk/files "/var/backups/sticktalk/$stamp/files"
sudo systemctl start sticktalk
```

If a command fails, restart the service and investigate. Download an encrypted
copy **off the VM**; it contains private messages and password hashes. Securely
back up `/etc/sticktalk.env` and Caddyfile too. Monitor `df -h`, prune only old
unneeded copies, and never delete your only good backup. These commands are
manual: set a reminder or create a reviewed root cron job for regular backups.

For example, after the backup above, create an encrypted archive that your SSH
user can download. GPG asks for a new backup passphrase locally in your terminal;
keep it safe and never share it in chat. Losing it makes the archive unreadable.

```bash
sudo apt-get install -y gnupg
set -o pipefail
sudo tar -C /var/backups/sticktalk -czf - "$stamp" | gpg --symmetric --cipher-algo AES256 --output "$HOME/sticktalk-$stamp.tar.gz.gpg"
chmod 600 "$HOME/sticktalk-$stamp.tar.gz.gpg"
```

**Windows:** replace the timestamp with the filename just created:

```powershell
scp -i "$HOME\Downloads\ssh-key.key" ubuntu@YOUR_PUBLIC_IP:sticktalk-YYYYMMDDTHHMMSSZ.tar.gz.gpg "$HOME\Documents\"
```

Confirm the download and test decrypting a copy before deleting any backups.
`gpg --decrypt ARCHIVE.tar.gz.gpg` decrypts to standard output; use it only in a
private recovery location, not a public terminal log.

Restore with the service stopped, into a fresh data directory: move existing
data aside for recovery, copy the snapshot DB and files, leave no stale `-wal`
or `-shm` files, and `sudo chown -R sticktalk:sticktalk /var/lib/sticktalk`.
Start and test login, old history and an attachment. Never overwrite a live DB.
Restoring an old snapshot intentionally loses newer changes.

## 7. Updating

Back up first. In the source folder run `git pull --ff-only`, then
`bash deploy/install.sh`. Review migration/release notes, then
`sudo systemctl restart sticktalk`. Test health/login/chat. Embedded migrations
run before listening. Downgrading across schema changes requires the matching
backup. Keep Ubuntu/Caddy patched with `sudo apt-get update && sudo apt-get
upgrade`; reboot when requested.

## Phase 2 voice: NOT enabled

**TODO:** document the str0m SFU's explicit UDP port or bounded range, public
advertised IP, NAT/ICE behavior and TURN requirements. Add matching **UDP** rules
to BOTH Oracle security lists/NSGs and Ubuntu iptables only when voice is
implemented. Caddy proxies HTTP/WebSocket, not media UDP. Do not open all UDP
ports now. Phase 1 voice commands return `unsupported_feature`.

