# Cairn architecture

This document describes how Cairn is built and how it behaves. It is the reference for the code: when the two
disagree, one of them has a bug.

## 1. Shape

```
                 browser
                    │  HTTPS (TLS ends at the reverse proxy)
                    ▼
            reverse proxy  ── optional ──►  OAuth broker (Tinyauth)
                    │                              │        and/or
                    │                              │   OIDC providers, called by Cairn directly
                    │                              │
                    ▼                              │ Remote-Email + X-Auth-Secret
         ┌─────────────────────┐                   │ on /auth/sso only
         │ cairn (one process) │◄──────────────────┘
         │  HTTP API + SPA     │
         └──────┬───────┬──────┘
                │       │
         /srv (files)  /data (cairn.db)
```

- One process, one static binary. The web front-end is compiled at build time and embedded in the binary.
- No background workers, no job queue, no cache server, no search index. Every request does its own work and ends.
- Almost no state about files in the database: no recent files, no starred files. The filesystem is the record.
  Two things are kept, both by path like share links: grants (who else may open a file or folder, section 7.1) and
  trash entries (what was deleted from where, section 7.2). Nothing follows a file that is moved outside Cairn; a
  grant or link to a path that is gone simply stops working.
- Two mounted volumes: `/srv` is the server root (the files people browse), `/data` holds the SQLite database.
  The database never lives under the server root, so no user can see or download it.
- TLS is the reverse proxy's job. Cairn speaks plain HTTP inside the container network.

## 2. Stack

| Part | Choice | Why |
| --- | --- | --- |
| Language | Rust, stable, edition 2024 | Small static binary, low memory, no runtime. |
| HTTP | `axum` on `tokio` (current-thread runtime by default) | Small, streaming bodies, tower middleware. One thread is enough on a low-end PC; `CAIRN_THREADS` raises it. |
| Database | SQLite through `rusqlite` with the `bundled` feature | No server, one file, easy backups. WAL mode, one writer connection behind a mutex, short transactions. |
| Password hash | `argon2` (Argon2id, m=7 MiB, t=5, p=1) | The lowest-memory row of the OWASP baseline, which rates all its rows as equal. RAM is held only while one hash runs (tens of ms) and at most 2 hashes run at once, so a burst of sign-ins peaks at 14 MiB. scrypt at the same rating needs 16 MiB and 5 times the CPU (N=2^14, r=8, p=5). Parameters are fixed and stored in the PHC string. |
| Outbound HTTP | `ureq` with `rustls` | Only for CAPTCHA checks, the OAuth broker and OIDC providers. Blocking client on `spawn_blocking`; no OpenSSL. Trusts Mozilla's CA list built into the binary plus the system's (section 3, `SSL_CERT_FILE`). |
| OIDC tokens | `jsonwebtoken` (RS256, ES256) | Verifies ID tokens against the provider's JWKS. Reuses the `ring` that `rustls` already brings, so it adds little to the binary. |
| Embedded assets | `rust-embed`, pre-compressed with gzip and brotli at build time | Served with `Content-Encoding` and a hashed file name, cached for a year. |
| Front-end | TypeScript + Preact, bundled with esbuild | About 4 kB of runtime. No CSS framework: plain CSS on the variables in `tokens.css`. |
| Logs | `tracing` with a JSON line formatter on stdout | `docker logs` is the log store. |

Target budget, checked in CI: binary under 12 MB, idle memory under 20 MB, front-end under 100 kB gzipped.

## 3. Configuration

All configuration comes from environment variables. Settings an admin changes in the UI (default theme,
thumbnails, trash days) are stored in the database (`settings` rows `default_theme`, `thumbnails` and `trash_days`,
changed with `PUT /api/settings`); everything that decides security or networking is environment only, so
the UI cannot be used to weaken it. The Server settings screen shows those values read-only.

Every secret also accepts `<NAME>_FILE`, a path to a file holding the value (Docker and Compose secrets). Setting
both is a startup error. One trailing newline is dropped.

Startup checks the whole configuration and exits with one clear message per problem before it opens a socket.

| Variable | Default | Meaning |
| --- | --- | --- |
| `CAIRN_ROOT` | `/srv` | Server root. Every user scope is a folder inside it. |
| `CAIRN_DATA` | `/data` | Folder for `cairn.db`. Must not be inside `CAIRN_ROOT`. |
| `CAIRN_ADDR` | `0.0.0.0:8080` | Listen address inside the container. |
| `CAIRN_PUBLIC_URL` | none | The address people use, e.g. `https://files.example.com`. Required for OAuth, OIDC and CAPTCHA, and used to build share links. Without it, share links use the address in the browser that made them, which may not open for people outside that network; the Share screens say so. |
| `CAIRN_TRUSTED_PROXIES` | none | Comma-separated CIDRs whose `X-Forwarded-For` and `X-Forwarded-Proto` are believed. Without it the socket peer is the client IP. Set it whenever Cairn runs behind a reverse proxy (section 4.6). |
| `CAIRN_AUTH_METHODS` | `password` | Comma-separated list of `password`, `oauth` (through a broker) and `oidc` (direct). Any combination; at least one. |
| `CAIRN_OAUTH_BROKER_URL` | none | Internal URL of the OAuth broker, e.g. `http://tinyauth:3000`. Required with `oauth`. |
| `CAIRN_OAUTH_SECRET` | none | Shared secret the broker sends as `X-Auth-Secret`. Required with `oauth`. At least 32 characters. |
| `CAIRN_OAUTH_LOGOUT_URL` | none | Broker sign-out page. Sign-out after an OAuth sign-in goes through it. |
| `CAIRN_OIDC_PROVIDERS` | none | With `oidc`: comma-separated provider ids, e.g. `google,authentik`. Each id `X` reads the four variables below. |
| `CAIRN_OIDC_X_ISSUER` | none | Issuer URL, e.g. `https://accounts.google.com`. Must be `https`. Discovery reads `/.well-known/openid-configuration` under it. |
| `CAIRN_OIDC_X_CLIENT_ID` | none | Client id registered with the provider. |
| `CAIRN_OIDC_X_CLIENT_SECRET` | none | Client secret. Accepts `_FILE`. |
| `CAIRN_OIDC_X_NAME` | the id | Button label, e.g. `Google`. |
| `CAIRN_CAPTCHA` | `none` | `none`, `recaptcha` (v2, checkbox), `turnstile` or `hcaptcha`. |
| `CAIRN_CAPTCHA_SITE_KEY` | none | Public widget key. Required when `CAIRN_CAPTCHA` is not `none`. |
| `CAIRN_CAPTCHA_SECRET` | none | Server-side key. Required when `CAIRN_CAPTCHA` is not `none`. |
| `CAIRN_UPLOAD_MAX` | `0` (no limit) | Largest accepted upload, in bytes. |
| `CAIRN_EDIT_MAX` | `2097152` | Largest text file the editor opens, in bytes. |
| `CAIRN_LOG` | `info` | `debug`, `info`, `warn` or `error`. |
| `CAIRN_THREADS` | `1` | Worker threads, 1 to 64. 1 runs a single-thread runtime. |
| `CAIRN_TRASH_DAYS` | none (the setting, default `30`) | Days a deleted item stays in the trash, 0 to 3650. `0` turns the trash off: deletes are permanent. When set, it wins over the setting in Server settings, which then shows it read-only. |
| `CAIRN_DEV_USER` | none | Development only: every request is this user, no sign-in. Refused unless the binary is a debug build. |
| `SSL_CERT_FILE`, `SSL_CERT_DIR` | the system's list | A PEM file (or folder) of extra certificate authorities, for an identity provider or broker whose certificate comes from your own CA. Cairn adds them to Mozilla's list built into the binary, so the file may hold only your CA; public sites (CAPTCHA vendors, Google) stay trusted. |

## 4. Sign-in

Four pieces, each optional, combined by configuration: a password form, OAuth through a broker, OIDC straight to
a provider, and a CAPTCHA on the password form. The two external routes exist side by side because they suit
different setups:

- **Broker (`oauth`).** A broker (Tinyauth is the documented one) holds the provider credentials and does the OAuth
  dance; Cairn only trusts the broker's word on one route, checked with a shared secret. Best when the operator
  already runs a broker for other apps: one place to add providers, one sign-in for every app.
- **Direct (`oidc`).** Cairn runs the OpenID Connect authorization code flow itself. Best for a single app, with no
  extra container and no forward-auth rule in the proxy. Needs a provider that speaks OIDC (Google, Microsoft,
  GitLab, Authentik, Keycloak, Authelia, Pocket ID and most others). Plain OAuth 2 providers without OIDC, such as
  GitHub, are reachable only through the broker.

Both may be on at the same time; the sign-in page then shows one button per provider from both lists. Whatever the
route, the outcome is the same: a verified e-mail address, matched to an existing user.

### 4.1 Identity

- The **e-mail** is the account. It is what a person types in the password form and what the broker or OIDC
  provider reports. Unique, trimmed and compared lower-cased. A user also has a display **name**, shown in the UI.
- A password is optional. A user without one signs in only through `oauth` or `oidc`. With neither of those
  enabled, a user without a password cannot sign in, so the Users screen and `cairn user add` require one.
- Nobody changes their own e-mail. An admin changing someone's e-mail hands the account (shares, preferences) to
  whoever signs in with the new address, ends that user's sessions and is logged as `users.email_reassigned`.
- OAuth and OIDC never create users. An e-mail the broker vouches for but that matches no user is refused with
  "No account for ana@example.com. Ask the person who runs this server to add you." Self-registration is out of
  scope.

### 4.2 Password

`POST /api/auth/login` with `{ email, password, captchaToken?, keep }`.

1. Rate limit check (section 4.6). Over the limit: `429` with `Retry-After`.
2. CAPTCHA check when enabled (section 4.5). Fails closed: a missing, wrong or unverifiable token is `400`.
3. Look up the user by e-mail. If there is no user or no password hash, verify against a fixed dummy hash anyway,
   so a wrong e-mail costs the same time as a wrong password.
4. Wrong: `401` with "Wrong e-mail or password." The answer never says which one.
5. Right: new session, cookie set, `200` with the public user.

Passwords: 8 to 256 characters, with at least one lowercase letter, one uppercase letter and one character that is
not a letter or digit. The error says the whole rule: "The password needs at least 8 characters, with an uppercase
letter, a lowercase letter and a special character." The hash string is
parsed strictly; only the fixed Argon2id parameters are accepted, so a crafted hash in a restored database cannot
make the server stall.

`401` is the only status a failed sign-in returns, and nothing else in Cairn answers `401` to a person typing.
That lets an operator point Fail2ban (or CrowdSec) at the JSON log line `auth.login_failed` without false hits.
Being signed out is not a failure: `GET /api/me` answers `200 { user: null }`.

### 4.3 OAuth through a broker

Enabled by `CAIRN_AUTH_METHODS` containing `oauth`. Two routes, both page navigations outside `/api`:

- `GET /auth/oauth/{provider}`: Cairn asks the broker (server to server) to open a pending OAuth session for
  `{provider}` with redirect `CAIRN_PUBLIC_URL/auth/sso`, relays the broker's cookies, and redirects the browser to
  the provider's authorize URL. The authorize URL must be `https`. Rate limited per IP. A provider that is not in
the broker's list (a button from before a restart, for example) goes back to `/login?error=sso`.
- `GET /auth/sso`: the only route the reverse proxy puts behind the broker's forward-auth. Cairn reads
  `Remote-Email` and `Remote-Name` only here and only when `X-Auth-Secret` matches `CAIRN_OAUTH_SECRET`
  (constant-time compare). Any session cookie the browser brought is deleted first, so a planted session cannot
  survive. Then the e-mail is matched to a user (section 4.1) and a session starts with method `oauth`.

Routing is exact and case-sensitive: `/auth/sso/` and `/AUTH/SSO` are `404`, because the proxy matches the exact
path and a variant would reach Cairn without passing the broker.

The provider buttons come from the broker's provider list, cached in memory for 60 seconds (20 seconds after a
failed fetch) and refreshed in the background. A slow or stopped broker never delays the sign-in page; the buttons
then show the last good list, or none.

Cairn speaks the Tinyauth v5 API: `GET /api/context/app` for the list (`auth.providers`, items `{ id, name, oauth }`,
only those with `oauth: true`) and `GET /api/oauth/url/{id}?redirect_uri=...` for the start (`{ url }` plus the
cookies the broker's callback needs; at most 8 are relayed). These are the broker's own endpoints, not a published
interface, so pin the broker image to a v5 tag. If they move, the buttons disappear and Cairn logs
`oauth.broker_unreachable` with a hint naming the API it expects.

A finished sign-in, through the broker or OIDC, redirects to `/`. Its session lasts 12 hours, without "Keep me
signed in": the provider's own session makes signing in again one click.

Sign-out after an OAuth sign-in answers `{ next: CAIRN_OAUTH_LOGOUT_URL }`, so the browser also ends the broker's
own session.

Errors come back to the sign-in page as a query value (`/login?error=sso` or `/login?error=no-account`) and are
shown in the form's error line. The e-mail of a refused account travels in a short-lived HttpOnly cookie read once,
never in the URL. The OIDC route below uses the same values and the same cookie.

### 4.4 OIDC, direct

Enabled by `CAIRN_AUTH_METHODS` containing `oidc`. Authorization code flow with PKCE (S256), confidential client.

- **Startup.** For each provider, Cairn fetches the discovery document and JWKS. A provider that cannot be reached
  is logged (`oidc.unreachable`) and its button is hidden; Cairn still starts and retries in the background every
  minute. A malformed configuration (missing client id, `http` issuer) stops startup instead.
- **`GET /auth/oidc/{id}`.** Creates `state`, `nonce` and a PKCE verifier, keeps them in a 10-minute HttpOnly cookie
  signed (HMAC-SHA256) with a key the process makes at start, so nobody can choose them and a restart ends sign-ins
  in progress. Each sign-in has its own cookie, named after the start of its `state`, so a second tab does not
  overwrite the first. Over HTTPS the name starts with `__Host-cairn_oidc-` (Secure, Path=/), so a sibling
  subdomain cannot plant one that Cairn gave it and sign a victim in as someone else; over plain HTTP it starts
  with `cairn_oidc-` and is scoped to `/auth/oidc/`. Then Cairn redirects to the provider's authorization endpoint with scope
  `openid email profile` and redirect `CAIRN_PUBLIC_URL/auth/oidc/{id}/callback`. Rate limited per IP, sharing the
  limit of the broker route.
- **`GET /auth/oidc/{id}/callback`.** Checks `state` against the cookie (each `state` works once), deletes the
  cookie, exchanges the code at
  the token endpoint (client secret in the body, `client_secret_post`, or Basic when discovery lists only that), and
  verifies the ID token:
  - signature with a key from the JWKS, algorithm RS256 or ES256 only (never `none`, never HS256);
  - `iss` equals the configured issuer (a trailing `/` aside; for `https://accounts.google.com` also the bare
    `accounts.google.com` that Google documents), `aud` contains the client id, `azp` (when present) is the client id;
  - `exp` in the future and `iat` not more than 5 minutes ahead, with 60 seconds of clock leeway;
  - `nonce` equals the one in the cookie;
  - `email` present and `email_verified` true. An unverified address is refused, because anyone can put any
    address on an account at some providers.
  An unknown `kid` triggers one JWKS refresh, at most once a minute, to follow key rotation.
- Then, as with the broker: delete any session the browser brought, match the e-mail to a user, start a session
  with method `oidc`. A callback refused before the provider's answer is trusted (no cookie, wrong or reused
  `state`, the provider's own error) leaves the browser's session alone, so reloading the callback after signing in
  does not sign anyone out.
- Cairn keeps no provider tokens. The access token is used for nothing and dropped; there is no refresh token.
- Sign-out ends only the Cairn session. The provider session stays, as with any "Sign in with" button. RP-initiated
  logout is left out on purpose: it would sign people out of their Google or Microsoft account everywhere.

### 4.5 CAPTCHA

Shown on the password form only (OAuth providers run their own checks). Turnstile, hCaptcha and reCAPTCHA v2 share
one contract: POST `secret`, `response` and `remoteip` to the vendor's verify URL and read `{ success, hostname }`.
Each vendor is a row of data: its verify URL and the hosts its widget needs in the Content-Security-Policy.

- The answer's `hostname` must equal the host of `CAIRN_PUBLIC_URL`; a token solved on another site is refused.
- Vendor unreachable or slow (5 s timeout): refused. The form says "The anti-robot check could not be reached. Try
  again in a minute."
- The widget script loads only on the sign-in page, and only when CAPTCHA is on. Cairn is one page, so when
  CAPTCHA is on the app's CSP adds the vendor's hosts to `script-src` and a matching `frame-src`. File and share
  routes keep their own strict CSP.
- The widget follows the theme (`data-theme` dark or light).

`CAIRN_CAPTCHA` defaults to `none`, because every vendor needs keys the operator must create. The documented
choice, used in every example, is reCAPTCHA v2 (checkbox). Turnstile and hCaptcha work the same way and are one
variable away.

When the password form is on and CAPTCHA is off, startup logs `startup.captcha_disabled` and admins see a quiet
notice in Server settings: "Sign-in has no CAPTCHA. Turn one on before you open Cairn to the internet."

### 4.6 Rate limits

In memory, fixed windows, bounded maps (oldest entries dropped past 10 000). The process is single, so no shared
store is needed; a restart clears them, which is acceptable.

| What | Limit |
| --- | --- |
| Failed sign-ins per IP | 10 per 15 min |
| Failed sign-ins per e-mail | 5 per 15 min; a success clears it |
| OAuth and OIDC starts per IP | 20 per 10 min |
| Wrong share password, per share and IP together | 10 per 15 min |
| Wrong current password on "Change password" | 5 per 15 min per user, answered `400`, not `401` |
| E-mails without an account asked for when sharing with people | 20 per 15 min per user; the whole request is refused and nothing from it is saved |

Over a limit Cairn does not check the password at all: it answers `429` with `Retry-After` and logs
`http.rate_limited`. A sign-in, link-password or current-password try counts before the password is checked, in the
same step as the limit check, so tries sent at the same moment cannot all slip past; a right password gives its try
back, so only failures count. On the OAuth and OIDC starts, which are page navigations, the `429` body is a
plain-text sentence. The limits are fixed; there is no switch to turn them off.

They only work when Cairn sees each person's real address. Behind a reverse proxy, set `CAIRN_TRUSTED_PROXIES` to
the proxy's network. Without it every request seems to come from the proxy: one person mistyping blocks sign-in
for everyone for 15 minutes, Fail2ban would ban the proxy, and the session cookie loses `Secure`. Cairn logs
`http.forwarded_ignored` once when it sees `X-Forwarded-For` from a proxy it does not trust.

**Fail2ban.** Count only `401` on `POST /api/auth/login` (the `auth.login_failed` log line) and keep Cairn out of
generic 401/403 jails. Use `maxretry` between 6 and 10 and a `findtime` of at least 15 minutes. A person mistyping
their own password then gets Cairn's `429` after 5 tries, before any ban, while one address trying many accounts is
banned before Cairn's own limit of 10 per address.

### 4.7 Sign-in page data

`GET /api/config` needs no session and answers what the sign-in page and the app need before anyone signs in:
`authMethods`, `captcha` (`{ provider, siteKey }` or null), `providers` (one `{ name, href }` per sign-in button),
`setupNeeded` (only while there are no users), the server default `theme` (`dark`, `light` or null for "match the
device"), `thumbnails` and `editMax`.

Admins read the environment on the Server screen through `GET /api/server`: root, public address, listen address,
sign-in methods, CAPTCHA vendor, upload and edit limits, threads, trusted proxies and log level. Never a secret or
an internal URL.

### 4.8 Sessions

- Token: 32 random bytes, base64url, in an HttpOnly, `SameSite=Lax` cookie. Over HTTPS the cookie is named
  `__Host-cairn_session` (Secure, Path=/, no Domain), so a sibling subdomain cannot plant one.
- The database stores only the SHA-256 of the token, with user, method (`password`, `oauth` or `oidc`), created and expiry
  times, and a short user-agent label for the Profile screen.
- "Keep me signed in" gives 30 days, sliding: the expiry moves forward when less than half is left. Without it the
  cookie has no `Max-Age` and the server row expires after 12 hours.
- Changing a password, removing a permission, changing a scope or deleting a user ends that user's other sessions.
- Every state-changing request needs the `X-Cairn` header (any value). Browsers do not send custom headers
  cross-site without a CORS preflight, which Cairn never allows, so this plus `SameSite=Lax` stops CSRF without a
  token dance. The one `POST` that comes from a plain form, the ZIP of chosen items, changes nothing and needs proof
  that it came from this site instead: `Sec-Fetch-Site: same-origin`, or, from browsers that do not send it, an
  `Origin` whose host is the one in `CAIRN_PUBLIC_URL` or in `Host`.

## 5. Files

### 5.1 Path safety

Every path from a client is relative to the user's scope. The server resolves it in one place, `resolve(scope, path)`:

1. Reject NUL bytes, empty segments, `.` and `..` segments, and names longer than 255 bytes.
2. Join to the scope folder, which is itself inside `CAIRN_ROOT`.
3. Open with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` on Linux, so a symlink cannot lead out of the
   scope even if it is created between the check and the use. Where `openat2` is missing (kernel before 5.6),
   canonicalize and check the prefix, and refuse to follow symlinks at all.

4. Refuse any path whose folder is the data folder seen another way. Two bind mounts can show one host folder at
   two paths (for example `./files:/srv` and `./files/.cairn:/data`), so `/srv/.cairn` would be the database folder
   and canonical paths cannot tell. At startup Cairn records the device and inode of `CAIRN_DATA`; `resolve`
   rejects every path component with the same pair.

Names on Linux are bytes and need not be UTF-8; old copies from Windows, ZIP files and USB sticks often carry
Latin-1 accents. Such entries are listed with U+FFFD in place of the unreadable bytes and a `rawName` field. In a
path, a segment that starts with NUL (`%00` in a URL) carries a name's raw bytes in base64url, and every route
accepts it, so these files can be opened, downloaded, renamed to a readable name and deleted. A real name cannot
contain NUL, so this never collides with one.

Symlinks inside the scope that point inside the scope work. Symlinks that point outside are listed with a "link
outside your folder" marker and cannot be opened.

Trash folders (section 7.2) are reserved the same way as the data folder: every one of Cairn's `.cairn-trash`
folders is refused by device and inode, so no listing shows it and no route reaches into it, through a link or
otherwise. Nothing can be created under the name `.cairn-trash` in the server root or at the top of a mount inside
it. Upload temporaries (`.cairn-upload-*`) are refused as names everywhere.

### 5.2 Operations

| Action | Route | Permission |
| --- | --- | --- |
| List folder | `GET /api/files/{path}` | read |
| Download file | `GET /api/raw/{path}` (supports `Range`, `If-None-Match`) | read |
| Download folder | `GET /api/zip/{path}` (streamed ZIP, store only, no compression) | read |
| Download chosen items | `POST /api/zip/{path}` with a form body `item=<name>&item=<name>` (1 to 1000 names inside that folder) | read |
| Upload | `PUT /api/upload/{path}` | write |
| New folder | `POST /api/folders/{path}` | write |
| Rename, move, copy | `POST /api/ops` with a list of `{ op, from, to }` | write |
| Delete | `DELETE /api/files` with `{ paths, permanent? }`: to the trash, or for good with `permanent: true` | write |
| Save text file | `PUT /api/text/{path}` with `If-Match` | write |
| List share links | `GET /api/shares` (own; admins all) | any |
| Create a share link | `POST /api/shares` with `{ path, expiresDays, password?, download }` | share |
| Change or delete a share link | `PATCH` / `DELETE /api/shares/{id}` | its creator, or an admin |
| Share with people | `POST /api/grants` with `{ path, emails, permission, expiresDays? }` | share (and write for "Can edit") |
| Grants given; received | `GET /api/grants` (own; admins all); `GET /api/grants/received` | any |
| Change or end a grant | `PATCH /api/grants/{id}` (owner who may share, or admin); `DELETE /api/grants/{id}` (owner, admin, or the recipient: Leave) | |
| Inside a grant | `GET /api/g/{id}`, then the routes above under `/api/g/{id}/…` | the grant's |
| Trash | `GET /api/trash`, `POST /api/trash/restore` and `DELETE /api/trash` with `{ ids }`, `POST /api/trash/empty` | write to the place it came from |

- Listings stream as JSON, sorted on the server (folders first, then natural name order), so the browser can window
  them without sorting 50 000 entries. Each entry: name, kind, size, modified time, and a MIME guess from the
  extension only (no file sniffing). For admins the listing also carries `space`, the used and total bytes of the
  disk that holds the scope.
- Search is a filter of the open folder's listing, in the browser. There is no search index and no server search.
- Several chosen items download as one ZIP. The list travels in a form body, not the URL, so a long selection never
  hits a proxy's URL length limit. The form is a plain HTML form, so the browser saves the archive as it streams.
- Uploads stream to a temporary file next to the target (`.cairn-upload-<random>`), then rename into place.
  Per-file progress, retry and cancel live in the browser. A cancelled or broken upload removes its temporary file.
  While a temporary file exists its path is recorded in an `uploads` table, so after a crash startup removes exactly
  those leftovers without walking the whole tree. Existing names are never overwritten silently: the client asks first
  and then sends `?replace=1`.
- A replaced file (upload with `?replace=1`, or a text save) keeps the original's permissions, without setuid,
  setgid and sticky bits. It keeps the original's owner when Cairn may set it: as root, or when Cairn runs as that
  owner (`user:` in Compose). Otherwise the new file belongs to the user running Cairn.
- Moves inside one filesystem are renames. Across filesystems they are copy then delete, and the response says how
  many items moved and which ones failed.
- Text editing uses the file's ETag (size plus modified time). A save whose `If-Match` no longer matches answers
  `412`, and the editor says "This file changed on the server since you opened it."
- Delete moves items to the trash (section 7.2) and needs a confirmation in the UI; "Delete permanently" is a second,
  separate choice. The API takes the list in the body, never a wildcard. A delete that cannot go to the trash is
  refused for that item (`trash_off`, `trash_taken`, `trash_denied`, `other_disk` or `holds_mount`), never silently made permanent.
- Overwriting a file (an upload with `?replace=1`, or a text save) is not a delete and does not go to the trash.
- Thumbnails, when the setting is on, are the original image served by `/api/raw` with `loading="lazy"`. No
  resizing and no cache on the server.

## 6. Users and permissions

- Fields: id, e-mail, name, password hash (optional), scope, permissions, preferred language, theme and view,
  created time.
- Scope is a folder relative to `CAIRN_ROOT`, written like `/photos`. `/` is the whole root.
- Permissions are four flags. `write` implies `read`. `share` needs `read`. `admin` gives the Users and Server
  screens and every link on the Shared links page; an admin's file access still follows their own scope.
- The last admin cannot be deleted or lose `admin`.
- First run: the first admin is created with the command line, never from environment variables and never with a
  default password. While the users table is empty, startup logs `startup.no_users` with the command to run, and
  the sign-in page shows "No users yet. Create the first admin with `cairn user add`; the server log has the
  command." Once a user exists, the page shows nothing about it.
- The command line is part of the same binary, so it works inside the `scratch` image with `docker exec`, which
  needs no shell. It opens the same database file, which WAL mode allows while the server runs.

```bash
# first admin, scope /, all permissions; asks for the password without echoing it
docker exec -it cairn /cairn user add "Admin" admin@example.com --admin --password
# a user who signs in only with OAuth or OIDC (no password)
docker exec -it cairn /cairn user add "Guest" guest@example.com --scope /public
# set or change a password (ends that user's sessions)
docker exec -it cairn /cairn user passwd guest@example.com
docker exec -it cairn /cairn user list
docker exec -it cairn /cairn user delete guest@example.com
```

  `--password` reads the password from the terminal without echo, or from stdin when piped, never from an argument
  (arguments show up in `ps` and shell history). Without `oauth` or `oidc` enabled, `add` refuses a user without
  `--password`. After the first admin, users are managed on the Users screen.

## 7. Share links

- A share is a random 22-character id (128 bits) pointing at one file or folder, with an expiry (1 day, 7 days,
  30 days or never), an optional password and a download switch. Creating one needs `share`; the target must lie
  inside the creator's scope. Each person sees and manages their own links on the Shared links page (`/links`,
  beside My files); admins see everyone's. Someone who lost `share` can still delete their links, but not change
  them.
- A link password needs 8 to 256 characters. Wrong guesses are limited per link and address (section 4.6), so a
  short password would fall to someone with many addresses.
- `GET /s/{id}` is the public page. With a password, the page asks for it and, when right, sets a cookie scoped to
  `/s/{id}` for 12 hours. The cookie carries an expiry and an HMAC over the link, the expiry and the password hash,
  under a key the process makes at start: changing or removing the password, or restarting Cairn, ends every unlock.
  A wrong password answers `403`, never `401`. Each try counts toward the limit before the password is checked, so
  tries sent at once cannot slip past it; the right password clears the count.
- The page's data lives under the same path, so the cookie reaches it: `GET /s/{id}/info` (the item, the download
  switch, the expiry and the creator's name), `POST /s/{id}/unlock`, and for files `GET /s/{id}/raw[/{path}]`,
  `GET /s/{id}/list[/{path}]`, `GET /s/{id}/info/{path}` (one item, for the page of a file inside) and
  `GET /s/{id}/zip[/{path}]` beneath a shared folder. A folder's archive is named after the folder. These routes need no session.
  They are matched exactly and case-sensitively, like `/auth`: any other path under `/s` is a plain-text `404`.
- With downloads off, the page lists the contents and previews text and images inline, but sends
  `Content-Disposition: inline` and no download buttons, and the ZIP route answers `403`. This is a convenience, not
  a protection: anything a browser shows can be saved, and the share screen says so.
- A share stops working when it expires, when it is deleted, when its creator loses `share`, or when its target
  leaves the creator's scope (moved, deleted, or the scope changed). Every case gives the visitor the same answer:
  "This link no longer works. Ask the person who sent it for a new one." Giving the permission or the scope back
  makes the link work again.
- A shared folder is opened as a scope of its own, so a symlink inside it reaches only what is beneath that folder,
  never its siblings in the creator's scope.
- `hits` counts openings of the public page (each answer of `info`); downloads are not counted.
- Expired shares are removed lazily, on the next write to the shares table. Deleting a user deletes their shares.
- Share ids are bearer secrets: they are never logged.

### 7.1 Sharing with people (grants)

- A grant lets one other Cairn user open one file or folder: owner, recipient, path (relative to `CAIRN_ROOT`, like
  a share's), permission (`read` or `write`), an expiry (1, 7 or 30 days, or never) and a creation time. Granting
  the same path to the same person again changes permission and expiry. Grant ids are random like share ids and
  never logged.
- Creating one needs `share` and `read`; "Can edit" also needs the owner's own `write`. Nobody grants to
  themselves. The Share dialog takes several e-mails at once and answers per address: added (with the person's
  name), "No account for x@example.com." or "That is you." E-mails are trimmed, compared lower-cased and counted
  once. Each one counts toward a limit of 20 per 15 minutes per person before it is looked up, and one with an
  account gives its try back, so only addresses without an account count (section 4.6); past the limit the
  request answers `429` and saves nothing. Typed addresses are never logged; `grant.unknown_email` logs a count.
- What the recipient may do: with `read`, list, preview, download and ZIP inside the item; with `write`, also
  create, upload, edit, rename, move, copy and delete inside it, whatever their own account's flags. The granted
  item itself cannot be renamed, moved or deleted by the recipient; a write grant on a single file changes its
  contents only (text save, upload with replace). A granted folder is opened as a scope of its own, so nothing
  outside it is reachable, symlinks included; a granted file is reached through its folder, limited to that one
  name.
- Deletes inside a grant always go to the trash. The recipient never deletes permanently: the API refuses
  `permanent: true` through a grant, and when the item cannot go to the trash the delete is refused with the
  reason.
- Nothing received is shared on: under `/api/g/{id}/…` only the file routes exist, so no link and no grant can be
  made from inside a grant.
- A grant stops working, checked on every request like a link, when it expires, when it is revoked or left, when
  its owner loses `share`, when its path leaves the owner's scope or is deleted, moved or renamed. A write grant
  works as read while its owner has no `write`. Every case that can come back (the permission, the scope, the path
  restored from the trash) makes the grant work again. Deleting either user deletes their grants. A dead grant
  answers `404 grant_gone`: "This shared item is no longer available."
- The recipient finds grants under Shared with me (`/shared`), opens them at `/shared/{id}` (then `/files/…` for a
  folder inside, `/view/…` for a file) and may Leave one. The owner manages them on the Shared links page, under
  People with access; admins see everyone's. The recipient never sees the owner's path, only the item's name.

### 7.2 Trash

- A delete moves the item, by `rename`, into `.cairn-trash/<entry id>/<original name>` at the top of the mount that
  holds it: `<CAIRN_ROOT>/.cairn-trash` for the root's own mount, and one per mount inside the root (two bind mounts
  of one disk are two mounts, and `rename` does not cross them). The mount is found by the kernel's mount id
  (`statx`, Linux 5.8), or on older kernels by the mount points in `/proc/self/mountinfo`. If the rename still
  fails with `EXDEV`, the item is refused with "{name} is on another disk, so it cannot go to the trash." and
  offered a permanent delete.
- An item that is a mount point, or holds one, is refused by both the trash and a permanent delete with "{name} has
  another disk mounted inside. Unmount it first." (`409 holds_mount`): the rename would carry the other disk into
  the trash, and the purge would later empty it.
- A trash folder is created on first use, mode `0700`, with a marker file naming this database: made beside under a
  temporary name, given its marker, then renamed into place, so it never exists without the marker. When Cairn may
  not write to the top of the mount, the delete is refused with `409 trash_denied` and offered a permanent delete. A `.cairn-trash`
  without that marker (a person's own folder, or one from another database) is never used, hidden or cleaned: the
  trash is off on that mount (`trash_taken`), startup logs `startup.trash_folder_taken`, and Server settings says
  to rename it.
- Each entry is a row in `trash`: id, its trash folder, its original path, name, kind, size, who deleted it, when,
  and when it goes. The row is written before the rename, so a crash leaves at most a row without a file, which
  reconciliation drops; files in a trash folder that no row names are counted, logged and left alone. Cairn
  never removes anything it did not put in the trash.
- An entry whose date has come is gone from every list and cannot be restored, even before the hourly purge
  removes it. Reconciliation and the purge wait for moves in flight, so they never drop a row whose item is on its
  way into the trash.
- Each entry keeps the removal date set when it was deleted: changing the days, or setting them to 0, affects later
  deletes only. With 0 nothing new enters; what is there stays until its date. Reconciliation and the purge run at
  start and every hour (`trash.purged` with a count).
- An entry is shown to every person who may now write to the place it came from: through their own scope (restore
  and delete permanently) or through a write grant on a folder holding it (restore only). Restore puts it back at
  its path, recreating missing folders; when the name is taken it comes back as `name (restored).ext`, then
  `name (restored 2).ext` (folders: `name (restored)`). If its folder is now on another disk, the restore is
  refused (`409 restore_other_disk`). The toast after a delete offers Undo, which restores.
- Server settings shows the days (or `CAIRN_TRASH_DAYS`), the total size of the entries and any trash folder that
  is not Cairn's.

## 8. Database

SQLite, one file, `PRAGMA journal_mode=WAL`, `foreign_keys=ON`. Migrations are numbered SQL files embedded in the
binary and run at startup inside one transaction.

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name          TEXT NOT NULL,
  password_hash TEXT,
  scope         TEXT NOT NULL DEFAULT '/',
  perm_read     INTEGER NOT NULL DEFAULT 1,
  perm_write    INTEGER NOT NULL DEFAULT 0,
  perm_share    INTEGER NOT NULL DEFAULT 0,
  perm_admin    INTEGER NOT NULL DEFAULT 0,
  lang          TEXT,
  theme         TEXT,
  view          TEXT,
  created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
  id         BLOB PRIMARY KEY,        -- SHA-256 of the token
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  method     TEXT NOT NULL,           -- 'password' | 'oauth' | 'oidc'
  keep       INTEGER NOT NULL DEFAULT 0, -- "Keep me signed in": 30 days, sliding
  label      TEXT,                    -- short user-agent label
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);

CREATE TABLE shares (
  id            TEXT PRIMARY KEY,
  user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  path          TEXT NOT NULL,        -- relative to CAIRN_ROOT
  password_hash TEXT,
  allow_download INTEGER NOT NULL DEFAULT 1,
  expires_at    INTEGER,
  created_at    INTEGER NOT NULL,
  hits          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE trash (
  id         TEXT PRIMARY KEY,
  trash_dir  TEXT NOT NULL,           -- the trash folder holding it, relative to CAIRN_ROOT
  path       TEXT NOT NULL,           -- where it was, relative to CAIRN_ROOT
  name       BLOB NOT NULL,           -- the original name's bytes
  kind       TEXT NOT NULL,
  size       INTEGER NOT NULL,
  deleted_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  deleted_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL         -- fixed when it was deleted
);

CREATE TABLE grants (
  id           TEXT PRIMARY KEY,
  owner_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  recipient_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  path         TEXT NOT NULL,         -- relative to CAIRN_ROOT
  write        INTEGER NOT NULL DEFAULT 0,
  expires_at   INTEGER,
  created_at   INTEGER NOT NULL,
  UNIQUE (owner_id, recipient_id, path)
);
```

The `settings` table also holds `trash_marker`, the token in every trash folder's marker.

Backup is copying `/data/cairn.db` while Cairn is stopped, or `cairn backup /data/backup.db` (SQLite online
backup) while it runs.

## 9. HTTP hardening

- Content-Security-Policy on every page: `default-src 'self'; img-src 'self' blob: data:; style-src 'self';
  script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`. When CAPTCHA is on it
  adds the vendor's hosts to `script-src` and a `frame-src` (section 4.5).
- Files served from `/api/raw` and `/s/` carry `X-Content-Type-Options: nosniff` and their own strict CSP
  (`default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; sandbox`), so an uploaded HTML or SVG file cannot
  run script in Cairn's origin. Files and ZIPs from `/s/` also carry `Cache-Control: private, no-cache`, so a shared
  cache never hands a password-protected file to someone without the cookie.
- `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`. HSTS is left to the reverse proxy.
- Request bodies other than uploads are capped at 1 MiB. JSON is parsed into fixed types; unknown fields are
  refused.
- Errors to clients are short sentences in the user's language. Details go to the log only.

## 10. Logging

One JSON object per line on stdout: time, level, event, and fields. Events are stable names, for example
`auth.login`, `auth.login_failed`, `auth.captcha_failed`, `auth.sso`, `auth.oidc`, `auth.oidc_failed`,
`auth.unknown_email`, `auth.sso_failed`, `auth.proxy_secret_mismatch`, `auth.oauth_start_failed`,
`oauth.broker_unreachable`, `oidc.ready`, `oidc.unreachable`, `startup.no_users`, `startup.captcha_disabled`, `file.delete`, `share.create`,
`share.update`, `share.delete`, `share.password_failed`, `grant.create`, `grant.update`, `grant.delete`,
`grant.unknown_email` (a count), `trash.move`, `trash.restore`, `trash.delete`, `trash.purged`, `trash.reconciled`,
`startup.trash_folder_taken`, `http.rate_limited` (with a `route`, such as `share_password` or `grant_lookup`). A failed sign-in logs the IP, the e-mail as typed (cut to 254 characters) and a reason
(`unknown_user`, `no_password`, `bad_password`). Passwords, tokens, share ids, grant ids, share passwords and e-mails typed to share with people are never logged.
The `auth.login_failed` line always begins with `timestamp`, `level`, `event` and `ip`, in that order: the
documented Fail2ban filter reads the address from that position, so typed text later in the line can never supply
it. Changing that order breaks every installed filter without a warning, so it is a breaking change for the
release notes; a test runs the real binary's line through the filter.

## 11. Interface language

Strings live in one table per language, compiled into the front-end bundle (English is the source; `pt-BR` is the
first translation). The server sends error messages as a code plus English text; the front-end shows the
translated text for known codes. The language comes from the user's Profile, then the browser's
`Accept-Language`, then English.

## 12. Docker image

The `Dockerfile` has three stages: the front-end with Node, the binary with Rust on Alpine (musl, so the binary is
static), then a `scratch` image that holds only `/cairn`, the CA certificates (the system's list, added to the one
built into the binary), `/etc/passwd` with one unprivileged user, and the empty folders `/srv` and `/data` owned by
that user. It runs as uid 10001, with `CAIRN_ROOT=/srv`, `CAIRN_DATA=/data` and `CAIRN_ADDR=0.0.0.0:8080`, exposes
8080, declares `/data` a volume, and carries the OCI labels for title, description and license (the release adds
source, version and revision). `scripts/check-image.sh` checks all of this on a built image.

The Rust stage builds for the architecture it runs on. Releases build `linux/amd64` on an x86_64 runner and
`linux/arm64` on an arm64 runner, never under emulation, check each with `scripts/check-image.sh`, and join them in
one manifest (`scripts/release-image.sh`, `.github/workflows/release.yml`). A tag `v<version>` must match the
version in `Cargo.toml`. The image is `ghcr.io/leodrivera/cairn` with the tags `<version>`, `<major>.<minor>` and
`latest`; a pre-release gets only its own version.

The image has a `HEALTHCHECK` that runs `/cairn health`, which calls `GET /api/health` on the local address and
exits 0 or 1, because `scratch` has no shell, `wget` or `curl`. To trust your own CA, mount its PEM file and set
`SSL_CERT_FILE` to its path.

The container runs as uid 10001. The mounted folders must be writable by that uid, or the operator sets `user:` in
Compose to match the owner of the files. When `/data` is not writable, startup stops with a message that names the
folder, the uid and both fixes.

### 12.1 Compose examples

The install guide (the README and `docs/install.md`) quotes the files in `examples/compose/`: `quick` (a home
network, plain HTTP), `traefik` (HTTPS and reCAPTCHA v2), `tinyauth` (the broker) and `oidc` (direct OIDC), plus the
add-ons `keycloak.yml` and `own-ca.yml` and the Fail2ban filter and jail. `tests/compose/run.sh` starts each one against an
image in a throwaway environment (a local CA for `*.example.com`, Traefik, Keycloak and Tinyauth) and checks what
the guide promises; `tests/compose/check-readme.sh` checks that the guide quotes the files exactly. Every service
the guide's commands name is called `cairn` (`container_name`), so the first admin is always:

```bash
docker exec -it cairn /cairn user add "Admin" admin@example.com --admin --password
```

## 13. Repository layout

```
cairn/
├── Cargo.toml
├── src/
│   ├── main.rs          # command line: serve, health, user, backup
│   ├── config.rs        # environment, *_FILE secrets, startup checks
│   ├── http/            # router, middleware, errors, static assets
│   ├── auth/            # password, oauth broker, oidc, captcha, sessions, rate limits
│   ├── fs/              # resolve, list, upload, ops, zip, text
│   ├── shares.rs
│   ├── grants.rs        # sharing with other users
│   ├── trash/           # trash folders, entries, move, restore, purge
│   ├── users.rs
│   ├── db/              # connection, migrations/*.sql
│   └── log.rs
├── web/                 # TypeScript + Preact front-end, built to web/dist
├── docker/              # passwd for the scratch image
├── Dockerfile
├── examples/compose/    # the Compose files of the install guide
├── scripts/             # budget, image, license and release checks
├── tests/compose/       # starts each example against an image
├── .github/workflows/   # CI on every push, the release on a tag
├── brand/               # logo and icons (favicon files are copied into web/dist)
├── docs/
└── tokens.css
```

## 14. Tests

- Unit tests for `resolve` with a fixture tree of symlinks, `..`, odd Unicode and long names. This function gets
  the most tests in the project.
- Auth tests with a fake broker and a fake CAPTCHA verifier: wrong secret, missing headers, path variants of
  `/auth/sso`, planted session, unknown e-mail, vendor timeout, wrong hostname. OIDC tests with a fake provider:
  wrong `state`, replayed `nonce`, `alg: none`, HS256 signed with the client secret, wrong `aud`, expired token,
  `email_verified` false, unknown `kid` then rotation.
- API tests against a temporary root and an in-memory database.
- A size and memory check in CI that fails the build past the budget in section 2.

## 15. Publishing hygiene

Cairn is meant to be published. The repository holds no personal data:

- Sample data uses `example.com` domains, generic names and placeholder keys.
- The image is published as `ghcr.io/leodrivera/cairn`, and the docs and examples use that name. A fork that
  publishes its own image changes it in the README, `docs/install.md`, `docs/architecture.md` and
  `examples/compose/`; the release workflow takes the owner from the repository.
- No real hostnames, e-mail addresses, IPs or secrets in code, docs, tests or commit messages. `.env` files and
  `secrets/` are in `.gitignore`.
- Brand files carry no editor metadata (checked: none of the files in `brand/` has author or path metadata).

## 16. License

AGPL-3.0-only. The full text is in `LICENSE`. Anyone who runs a modified Cairn for others over a network must
offer them its source. Everything that ships inside the binary is under licenses AGPL-3.0 accepts: MIT, Apache-2.0,
ISC, BSD-3-Clause, Zlib, Unicode-3.0 (Unicode data tables) and CDLA-Permissive-2.0 (the built-in list of certificate
authorities). `scripts/check-licenses.sh` checks this in CI and fails on anything else.
