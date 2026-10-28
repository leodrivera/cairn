# Cairn architecture

Status: design. This document describes how Cairn is meant to be built. Nothing here is implemented yet.

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
- No state about files in the database: no recent files, no starred files, no trash. The filesystem is the only
  record, so files moved outside Cairn (over SMB, from a terminal) never leave stale entries behind.
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
| Outbound HTTP | `ureq` with `rustls` | Only for CAPTCHA checks, the OAuth broker and OIDC providers. Blocking client on `spawn_blocking`; no OpenSSL. |
| OIDC tokens | `jsonwebtoken` (RS256, ES256) | Verifies ID tokens against the provider's JWKS. Reuses the `ring` that `rustls` already brings, so it adds little to the binary. |
| Embedded assets | `rust-embed`, pre-compressed with gzip and brotli at build time | Served with `Content-Encoding` and a hashed file name, cached for a year. |
| Front-end | TypeScript + Preact, bundled with esbuild | About 4 kB of runtime. No CSS framework: plain CSS on the variables in `tokens.css`. |
| Logs | `tracing` with a JSON line formatter on stdout | `docker logs` is the log store. |

Target budget, checked in CI: binary under 12 MB, idle memory under 20 MB, front-end under 100 kB gzipped.

## 3. Configuration

All configuration comes from environment variables. Settings an admin changes in the UI (default theme,
thumbnails) are stored in the database; everything that decides security or networking is environment only, so
the UI cannot be used to weaken it. The Server settings screen shows those values read-only.

Every secret also accepts `<NAME>_FILE`, a path to a file holding the value (Docker and Compose secrets). Setting
both is a startup error. One trailing newline is dropped.

Startup checks the whole configuration and exits with one clear message per problem before it opens a socket.

| Variable | Default | Meaning |
| --- | --- | --- |
| `CAIRN_ROOT` | `/srv` | Server root. Every user scope is a folder inside it. |
| `CAIRN_DATA` | `/data` | Folder for `cairn.db`. Must not be inside `CAIRN_ROOT`. |
| `CAIRN_ADDR` | `0.0.0.0:8080` | Listen address inside the container. |
| `CAIRN_PUBLIC_URL` | none | The address people use, e.g. `https://files.example.com`. Required for OAuth, OIDC and CAPTCHA, and used to build share links. |
| `CAIRN_TRUSTED_PROXIES` | none | Comma-separated CIDRs whose `X-Forwarded-For` and `X-Forwarded-Proto` are believed. Without it the socket peer is the client IP. |
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
| `CAIRN_DEV_USER` | none | Development only: every request is this user, no sign-in. Refused unless the binary is a debug build. |

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
  the provider's authorize URL. The authorize URL must be `https`. Rate limited per IP.
- `GET /auth/sso`: the only route the reverse proxy puts behind the broker's forward-auth. Cairn reads
  `Remote-Email` and `Remote-Name` only here and only when `X-Auth-Secret` matches `CAIRN_OAUTH_SECRET`
  (constant-time compare). Any session cookie the browser brought is deleted first, so a planted session cannot
  survive. Then the e-mail is matched to a user (section 4.1) and a session starts with method `oauth`.

Routing is exact and case-sensitive: `/auth/sso/` and `/AUTH/SSO` are `404`, because the proxy matches the exact
path and a variant would reach Cairn without passing the broker.

The provider buttons come from the broker's provider list, cached in memory for 60 seconds and refreshed in the
background. A slow or stopped broker never delays the sign-in page; the buttons then show the last good list, or
none.

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
  scoped to `/auth/oidc/`, and redirects to the provider's authorization endpoint with scope
  `openid email profile` and redirect `CAIRN_PUBLIC_URL/auth/oidc/{id}/callback`. Rate limited per IP, sharing the
  limit of the broker route.
- **`GET /auth/oidc/{id}/callback`.** Checks `state` against the cookie, deletes the cookie, exchanges the code at
  the token endpoint (client secret in the body, `client_secret_post`, or Basic when discovery lists only that), and
  verifies the ID token:
  - signature with a key from the JWKS, algorithm RS256 or ES256 only (never `none`, never HS256);
  - `iss` equals the configured issuer, `aud` contains the client id, `azp` (when present) is the client id;
  - `exp` in the future and `iat` not more than 5 minutes ahead, with 60 seconds of clock leeway;
  - `nonce` equals the one in the cookie;
  - `email` present and `email_verified` true. An unverified address is refused, because anyone can put any
    address on an account at some providers.
  An unknown `kid` triggers one JWKS refresh, at most once a minute, to follow key rotation.
- Then, as with the broker: delete any session the browser brought, match the e-mail to a user, start a session
  with method `oidc`.
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
- The widget script loads only on the sign-in page, and only when CAPTCHA is on. The CSP for that page adds the
  vendor's hosts; every other page keeps the strict default.
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
| Wrong share password per share and per IP | 10 per 15 min |
| Wrong current password on "Change password" | 5 per 15 min per user, answered `400`, not `401` |

### 4.7 Sessions

- Token: 32 random bytes, base64url, in an HttpOnly, `SameSite=Lax` cookie. Over HTTPS the cookie is named
  `__Host-cairn_session` (Secure, Path=/, no Domain), so a sibling subdomain cannot plant one.
- The database stores only the SHA-256 of the token, with user, method (`password`, `oauth` or `oidc`), created and expiry
  times, and a short user-agent label for the Profile screen.
- "Keep me signed in" gives 30 days, sliding: the expiry moves forward when less than half is left. Without it the
  cookie has no `Max-Age` and the server row expires after 12 hours.
- Changing a password, removing a permission, changing a scope or deleting a user ends that user's other sessions.
- Every state-changing request needs the `X-Cairn` header (any value). Browsers do not send custom headers
  cross-site without a CORS preflight, which Cairn never allows, so this plus `SameSite=Lax` stops CSRF without a
  token dance.

## 5. Files

### 5.1 Path safety

Every path from a client is relative to the user's scope. The server resolves it in one place, `resolve(scope, path)`:

1. Reject NUL bytes, empty segments, `.` and `..` segments, and names longer than 255 bytes.
2. Join to the scope folder, which is itself inside `CAIRN_ROOT`.
3. Open with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` on Linux, so a symlink cannot lead out of the
   scope even if it is created between the check and the use. Where `openat2` is missing (kernel before 5.6),
   canonicalize and check the prefix, and refuse to follow symlinks at all.

Symlinks inside the scope that point inside the scope work. Symlinks that point outside are listed with a "link
outside your folder" marker and cannot be opened.

### 5.2 Operations

| Action | Route | Permission |
| --- | --- | --- |
| List folder | `GET /api/files/{path}` | read |
| Download file | `GET /api/raw/{path}` (supports `Range`, `If-None-Match`) | read |
| Download folder | `GET /api/zip/{path}` (streamed ZIP, store only, no compression) | read |
| Upload | `PUT /api/upload/{path}` | write |
| New folder | `POST /api/folders/{path}` | write |
| Rename, move, copy | `POST /api/ops` with a list of `{ op, from, to }` | write |
| Delete | `DELETE /api/files` with a list of paths | write |
| Save text file | `PUT /api/text/{path}` with `If-Match` | write |

- Listings stream as JSON, sorted on the server (folders first, then natural name order), so the browser can window
  them without sorting 50 000 entries. Each entry: name, kind, size, modified time, and a MIME guess from the
  extension only (no file sniffing).
- Uploads stream to a temporary file next to the target (`.cairn-upload-<random>`), then rename into place.
  Per-file progress, retry and cancel live in the browser. A cancelled or broken upload removes its temporary file;
  startup removes leftovers older than a day. Existing names are never overwritten silently: the client asks first
  and then sends `?replace=1`.
- Moves inside one filesystem are renames. Across filesystems they are copy then delete, and the response says how
  many items moved and which ones failed.
- Text editing uses the file's ETag (size plus modified time). A save whose `If-Match` no longer matches answers
  `412`, and the editor says "This file changed on the server since you opened it."
- Delete is permanent and needs a confirmation in the UI. The API takes the list in the body, never a wildcard.
- Thumbnails, when the setting is on, are the original image served by `/api/raw` with `loading="lazy"`. No
  resizing and no cache on the server.

## 6. Users and permissions

- Fields: id, e-mail, name, password hash (optional), scope, permissions, preferred language, theme and view,
  created time.
- Scope is a folder relative to `CAIRN_ROOT`, written like `/photos`. `/` is the whole root.
- Permissions are four flags. `write` implies `read`. `share` needs `read`. `admin` gives the Users, Share links and
  Server screens; an admin's file access still follows their own scope.
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
  30 days or never), an optional password and a download switch.
- `GET /s/{id}` is the public page. With a password, the page asks for it and, when right, sets a cookie scoped to
  `/s/{id}` for 12 hours.
- With downloads off, the page lists the contents and previews text and images inline, but sends
  `Content-Disposition: inline` and no download buttons. This is a convenience, not a protection: anything a browser
  shows can be saved, and the share screen says so.
- A share stops working when it expires, when it is deleted, when its creator loses `share`, or when its target
  leaves the creator's scope.
- Expired shares are removed lazily, on the next write to the shares table.

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
```

Backup is copying `/data/cairn.db` while Cairn is stopped, or `cairn backup /data/backup.db` (SQLite online
backup) while it runs.

## 9. HTTP hardening

- Content-Security-Policy on every page: `default-src 'self'; img-src 'self' blob: data:; style-src 'self';
  script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`. The sign-in page adds the CAPTCHA
  vendor's hosts when CAPTCHA is on.
- Files served from `/api/raw` and `/s/` carry `X-Content-Type-Options: nosniff` and their own strict CSP
  (`default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; sandbox`), so an uploaded HTML or SVG file cannot
  run script in Cairn's origin.
- `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`. HSTS is left to the reverse proxy.
- Request bodies other than uploads are capped at 1 MiB. JSON is parsed into fixed types; unknown fields are
  refused.
- Errors to clients are short sentences in the user's language. Details go to the log only.

## 10. Logging

One JSON object per line on stdout: time, level, event, and fields. Events are stable names, for example
`auth.login`, `auth.login_failed`, `auth.captcha_failed`, `auth.sso`, `auth.oidc`, `auth.oidc_failed`,
`auth.unknown_email`, `startup.no_users`, `startup.captcha_disabled`, `file.delete`, `share.create`,
`http.rate_limited`. A failed sign-in logs the IP, the e-mail as typed (cut to 254 characters) and a reason
(`unknown_user`, `no_password`, `bad_password`). Passwords, tokens and share passwords are never logged.

## 11. Interface language

Strings live in one table per language, compiled into the front-end bundle (English is the source; `pt-BR` is the
first translation). The server sends error messages as a code plus English text; the front-end shows the
translated text for known codes. The language comes from the user's Profile, then the browser's
`Accept-Language`, then English.

## 12. Docker image

Multi-stage build: the front-end with Node, the binary with Rust for `x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`, then a `scratch` image that holds only the binary, CA certificates (for CAPTCHA and
broker calls over HTTPS) and `/etc/passwd` with one unprivileged user.

```dockerfile
FROM node:24-alpine AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund --ignore-scripts
COPY web ./
RUN npm run build

FROM rust:1-alpine AS bin
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --locked && strip target/release/cairn

FROM scratch
COPY --from=bin /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=bin /src/target/release/cairn /cairn
COPY docker/passwd /etc/passwd
USER 10001:10001
ENV CAIRN_ROOT=/srv CAIRN_DATA=/data CAIRN_ADDR=0.0.0.0:8080
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD ["/cairn", "health"]
ENTRYPOINT ["/cairn"]
CMD ["serve"]
```

`cairn health` calls `GET /api/health` on the local address and exits 0 or 1, because `scratch` has no shell, `wget`
or `curl`.

The container runs as uid 10001. The mounted folders must be writable by that uid, or the operator sets `user:` in
Compose to match the owner of the files.

### 12.1 Compose examples

Password only, with reCAPTCHA:

```yaml
services:
  cairn:
    image: ghcr.io/OWNER/cairn:latest
    restart: unless-stopped
    volumes:
      - /path/to/files:/srv
      - cairn-data:/data
    environment:
      CAIRN_PUBLIC_URL: https://files.example.com
      CAIRN_TRUSTED_PROXIES: 172.16.0.0/12
      CAIRN_CAPTCHA: recaptcha
      CAIRN_CAPTCHA_SITE_KEY: 6Lc...
      CAIRN_CAPTCHA_SECRET_FILE: /run/secrets/captcha_secret
    secrets: [captcha_secret]

volumes:
  cairn-data:

secrets:
  captcha_secret:
    file: ./secrets/captcha_secret
```

Then create the first admin: `docker exec -it cairn /cairn user add "Admin" admin@example.com --admin --password`.

Password and OAuth, behind Traefik with Tinyauth. Only `/auth/sso` goes through the broker's forward-auth; every
other path reaches Cairn directly.

```yaml
services:
  cairn:
    image: ghcr.io/OWNER/cairn:latest
    volumes:
      - /path/to/files:/srv
      - cairn-data:/data
    environment:
      CAIRN_PUBLIC_URL: https://files.example.com
      CAIRN_TRUSTED_PROXIES: 172.16.0.0/12
      CAIRN_AUTH_METHODS: password,oauth
      CAIRN_OAUTH_BROKER_URL: http://tinyauth:3000
      CAIRN_OAUTH_SECRET_FILE: /run/secrets/oauth_secret
      CAIRN_OAUTH_LOGOUT_URL: https://auth.example.com/logout
      CAIRN_CAPTCHA: recaptcha
      CAIRN_CAPTCHA_SITE_KEY: 6Lc...
      CAIRN_CAPTCHA_SECRET_FILE: /run/secrets/captcha_secret
    secrets: [oauth_secret, captcha_secret]
    labels:
      traefik.enable: "true"
      traefik.http.routers.cairn.rule: Host(`files.example.com`)
      traefik.http.routers.cairn-sso.rule: Host(`files.example.com`) && Path(`/auth/sso`)
      traefik.http.routers.cairn-sso.middlewares: tinyauth
      traefik.http.services.cairn.loadbalancer.server.port: "8080"
```

The broker must send `X-Auth-Secret` with the same value as `CAIRN_OAUTH_SECRET` on its forward-auth responses.

Password and direct OIDC, no broker. Register `https://files.example.com/auth/oidc/google/callback` as the
redirect URI at the provider.

```yaml
    environment:
      CAIRN_PUBLIC_URL: https://files.example.com
      CAIRN_AUTH_METHODS: password,oidc
      CAIRN_OIDC_PROVIDERS: google
      CAIRN_OIDC_GOOGLE_ISSUER: https://accounts.google.com
      CAIRN_OIDC_GOOGLE_CLIENT_ID: 1234567890-abc.apps.googleusercontent.com
      CAIRN_OIDC_GOOGLE_CLIENT_SECRET_FILE: /run/secrets/google_secret
      CAIRN_OIDC_GOOGLE_NAME: Google
      CAIRN_CAPTCHA: recaptcha
      CAIRN_CAPTCHA_SITE_KEY: 6Lc...
      CAIRN_CAPTCHA_SECRET_FILE: /run/secrets/captcha_secret
```
The README will carry a full, tested example once there is a release.

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
│   ├── users.rs
│   ├── db/              # connection, migrations/*.sql
│   └── log.rs
├── web/                 # TypeScript + Preact front-end, built to web/dist
├── docker/              # passwd for the scratch image
├── Dockerfile
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
- Docs use `ghcr.io/OWNER/cairn`, with `OWNER` standing for the publishing account.
- No real hostnames, e-mail addresses, IPs or secrets in code, docs, tests or commit messages. `.env` files and
  `secrets/` are in `.gitignore`.
- Brand files carry no editor metadata (checked: none of the files in `brand/` has author or path metadata).

## 16. License

AGPL-3.0-only. The full text is in `LICENSE`. Anyone who runs a modified Cairn for others over a network must
offer them its source. Every planned dependency is under MIT or Apache-2.0, which AGPL-3.0 accepts.
