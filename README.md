<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/cairn-logo-on-dark.svg">
    <img src="brand/cairn-logo-on-light.svg" height="48" alt="cairn">
  </picture>
</h1>

<p align="center"><strong>Your files. One small binary.</strong></p>

Cairn is a self-hosted web file browser written in Rust. Point it at a folder on your server and browse, upload, edit and share its contents from any device. It is built to stay out of the way: small enough to run comfortably on an old PC.

## Why

A cairn is a stack of stones that marks a trail. Cairn stacks your folders and files where you can find them again.

## Features

- Browse files in a list or gallery view
- Upload with per-file progress, retry and cancel
- Rename, move, copy and delete
- View and edit text files in the browser
- Optional image thumbnails: the browser loads the original file, and Cairn does no resizing
- Share links with expiry, optional password and download control
- Multiple users, each limited to a scope (a folder) with read, write, share and admin permissions
- Sign in with an e-mail and password, with OAuth providers through a broker such as Tinyauth, with any OpenID Connect provider directly, or any mix of these
- Optional CAPTCHA on the password form: reCAPTCHA v2, Cloudflare Turnstile or hCaptcha
- Light and dark themes, usable on desktop and phone
- The interface is in English and Português (Brasil), chosen in Profile
- Single static binary, no runtime to install, shipped as a small Docker image

## Non-goals

These keep Cairn small. Each one needs extra modules or state that a light server should not carry.

- No terminal or command runner
- No trash: deletes are permanent, behind a confirmation
- No recent or starred files: Cairn keeps no state about your files, so nothing goes stale when you move them outside it
- No video preview, and no server-side image processing

## Installation

Cairn ships as a Docker image for x86_64 and aarch64 (64-bit ARM, such as a Raspberry Pi 4 or 5):
`ghcr.io/leodrivera/cairn`. Tags: `0.1.0` for one exact release, `0.1` for the newest `0.1.x` (the examples use this
one, so `docker compose pull` brings fixes), and `latest`.

The image holds only the binary and a list of certificate authorities. It runs as uid 10001. Mount the folder to
browse at `/srv` and a volume for the database at `/data`, and configure it with environment variables; every
variable is in [`docs/architecture.md`](docs/architecture.md#3-configuration).

Every example below is a file in [`examples/compose/`](examples/compose), and each one was started and checked
against the image by `tests/compose/run.sh` before this release.

### Quick start on a home network

Plain HTTP on port 8080, password sign-in only. Docker opens published ports in the firewall itself, so ufw or firewalld rules do not close them: on a machine with a public address (a VPS, or a server with ports forwarded to it), anyone could reach Cairn without HTTPS or a CAPTCHA. Use the Traefik example there instead.

<!-- file: examples/compose/quick/compose.yml -->
```yaml
# Cairn on a home network, over plain HTTP on port 8080.
# Not for a machine with a public address (a VPS, a server with ports forwarded): Docker opens published ports in
# the firewall itself, so ufw or firewalld rules do not close them, and anyone could reach Cairn without HTTPS or a
# CAPTCHA. For that, use the Traefik example.
# Before the first start: mkdir files && sudo chown 10001:10001 files
services:
  cairn:
    image: ghcr.io/leodrivera/cairn:0.1
    container_name: cairn
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - ./files:/srv
      - cairn-data:/data

volumes:
  cairn-data:
```

The container writes to `files` as uid 10001, so give the folder to that uid first:

```bash
mkdir files
sudo chown 10001:10001 files
docker compose up -d
```

Then create the first admin. The command asks for the password without showing it:

```bash
docker exec -it cairn /cairn user add "Admin" admin@example.com --admin --password
```

Open `http://<this machine>:8080` and sign in. After the first admin, add people on the Users screen.

If the container keeps restarting, `docker logs cairn` says why: most often a folder or a secret file that uid
10001 cannot read or write, named in the message with what to change.

To serve files that belong to your own user instead (your home folder, say), run Cairn as that user: add
`user: "1000:1000"` (your `id -u` and `id -g`) to the `cairn` service, and mount a folder you own at `/data` in
place of the volume.

### Behind Traefik, with HTTPS and reCAPTCHA

For a server reachable from the internet. Traefik gets a certificate from Let's Encrypt for `files.example.com`
(point that name at the server and open ports 80 and 443), and the password form gets a reCAPTCHA v2 checkbox.
Create the keys at [google.com/recaptcha/admin](https://www.google.com/recaptcha/admin) for your domain.

<!-- file: examples/compose/traefik/compose.yml -->
```yaml
# Cairn behind Traefik, with HTTPS from Let's Encrypt and reCAPTCHA v2 on the password form.
# files.example.com must point at this machine, with ports 80 and 443 open to the internet.
# Before the first start:
#   mkdir files secrets && sudo chown 10001:10001 files
#   printf '%s' 'your-recaptcha-secret-key' > secrets/captcha_secret
#   sudo chown 10001:10001 secrets/captcha_secret && sudo chmod 400 secrets/captcha_secret
services:
  traefik:
    image: traefik:v3.7
    restart: unless-stopped
    command:
      - --providers.docker.exposedbydefault=false
      - --entrypoints.web.address=:80
      - --entrypoints.web.http.redirections.entrypoint.to=websecure
      - --entrypoints.web.http.redirections.entrypoint.scheme=https
      - --entrypoints.websecure.address=:443
      - --entrypoints.websecure.http.tls.certresolver=le
      - --certificatesresolvers.le.acme.email=admin@example.com
      - --certificatesresolvers.le.acme.storage=/letsencrypt/acme.json
      - --certificatesresolvers.le.acme.httpchallenge.entrypoint=web
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - letsencrypt:/letsencrypt

  cairn:
    image: ghcr.io/leodrivera/cairn:0.1
    container_name: cairn
    restart: unless-stopped
    volumes:
      - ./files:/srv
      - cairn-data:/data
    environment:
      CAIRN_PUBLIC_URL: https://files.example.com
      # The network below: requests from Traefik carry the visitor's address.
      CAIRN_TRUSTED_PROXIES: 172.31.0.0/24
      CAIRN_CAPTCHA: recaptcha
      CAIRN_CAPTCHA_SITE_KEY: 6Lc-your-site-key
      CAIRN_CAPTCHA_SECRET_FILE: /run/secrets/captcha_secret
    secrets:
      - captcha_secret
    labels:
      traefik.enable: "true"
      traefik.http.routers.cairn.rule: Host(`files.example.com`)
      traefik.http.routers.cairn.entrypoints: websecure
      traefik.http.services.cairn.loadbalancer.server.port: "8080"

networks:
  default:
    ipam:
      config:
        # Any free private range; CAIRN_TRUSTED_PROXIES must name the same one.
        - subnet: 172.31.0.0/24

volumes:
  cairn-data:
  letsencrypt:

secrets:
  captcha_secret:
    file: ./secrets/captcha_secret
```

Cairn runs as uid 10001 and reads the secret file as that user. Give the file to it, readable by nobody else:

```bash
sudo chown 10001:10001 secrets/captcha_secret && sudo chmod 400 secrets/captcha_secret
```

Two settings matter behind any reverse proxy:

- `CAIRN_PUBLIC_URL` is the address people use. Share links are built from it. Without it, a link uses the address
  in the browser that made it, which may not open for people outside your network; the Share screens say so.
- `CAIRN_TRUSTED_PROXIES` is the proxy's network. Without it every request seems to come from the proxy: one person
  mistyping a password blocks sign-in for everyone for 15 minutes, and Fail2ban would ban the proxy itself. Cairn
  logs `http.forwarded_ignored` once when it sees a proxy it does not trust. The example pins the Compose network
  to `172.31.0.0/24` so the two always match; pick another private range if that one is taken.

### Sign in through Tinyauth

[`examples/compose/tinyauth/compose.yml`](examples/compose/tinyauth/compose.yml) adds Tinyauth, an OAuth broker,
next to the password form. Use it when you already run Tinyauth for other apps, or for providers without OpenID
Connect, such as GitHub.

- Pin Tinyauth to its `v5` tag: Cairn speaks the v5 API.
- Only the exact path `/auth/sso` goes through Tinyauth's forward-auth (`http://tinyauth:3000/api/auth/traefik`,
  passing on `Remote-Email` and `Remote-Name`). Every other path reaches Cairn directly.
- Traefik adds `X-Auth-Secret` on that path only, because Tinyauth does not send it. Its value and
  `CAIRN_OAUTH_SECRET` both come from `CAIRN_SSO_SECRET` in `.env`, at least 32 characters.
- At your OAuth provider, the redirect URL is `https://auth.example.com/api/oauth/callback/<id>`, where `<id>` is the
  provider's name in Tinyauth's variables (`keycloak` in the example).
- Tinyauth never creates Cairn users. Someone the provider vouches for without an account sees "No account for
  ... Ask the person who runs this server to add you." Add them on the Users screen first, with or without a
  password.

### Sign in with OpenID Connect

[`examples/compose/oidc/compose.yml`](examples/compose/oidc/compose.yml) adds "Sign in with Google" through OpenID
Connect, run by Cairn itself: no broker, no forward-auth. Any provider that speaks OIDC works the same way
(Microsoft, GitLab, Authentik, Keycloak, Authelia, Pocket ID): each provider id `X` in `CAIRN_OIDC_PROVIDERS`
reads `CAIRN_OIDC_X_ISSUER`, `CAIRN_OIDC_X_CLIENT_ID`, `CAIRN_OIDC_X_CLIENT_SECRET_FILE` and `CAIRN_OIDC_X_NAME`.

- Register `https://files.example.com/auth/oidc/<id>/callback` as the redirect URI at the provider
  (`<id>` is `google` in the example).
- The issuer must be `https`.
- The provider must report a verified e-mail address; Cairn refuses an unverified one.

Cairn reads both secret files as uid 10001:

```bash
sudo chown 10001:10001 secrets/captcha_secret secrets/oidc_client_secret && sudo chmod 400 secrets/captcha_secret secrets/oidc_client_secret
```

To run your own Keycloak at `id.example.com` behind the same Traefik, add
[`examples/compose/keycloak.yml`](examples/compose/keycloak.yml). It sets the two variables Keycloak needs behind a
proxy: `KC_HOSTNAME` with its public address, and `KC_PROXY_HEADERS=xforwarded`. Before the first start,
set `KC_ADMIN` and `KC_ADMIN_PASSWORD` in `.env`: they are the first sign-in to Keycloak's admin console.

```bash
docker compose -f compose.yml -f ../keycloak.yml up -d
```

### A provider with a certificate from your own CA

When Keycloak, Tinyauth or another provider uses a certificate from your own certificate authority, put the CA's
certificate next to `compose.yml` as `ca.pem` and add this file. Cairn adds that CA to the list built into the
binary, so the file may hold only your CA and public sites stay trusted.

<!-- file: examples/compose/own-ca.yml -->
```yaml
# Trust your own certificate authority for Cairn's outbound HTTPS: an OIDC provider or a broker on a home network
# whose certificate your own CA signed. Put the CA's certificate (PEM) next to compose.yml as ca.pem, then:
#   docker compose -f compose.yml -f ../own-ca.yml up -d
# Cairn adds it to the list built into the binary, so the file may hold only your CA; public sites stay trusted.
services:
  cairn:
    volumes:
      - ./ca.pem:/etc/cairn/ca.pem:ro
    environment:
      SSL_CERT_FILE: /etc/cairn/ca.pem
```

### First start

When everything starts at once, the log may show `oidc.unreachable` or `oauth.broker_unreachable`: Cairn came up
before the provider or the broker. That is normal. Cairn asks an OIDC provider again every minute, and asks the
broker again 20 seconds after a failed try; the sign-in buttons appear then.

### Backups and restore

The database holds users, sessions, settings and share links; your files stay in their folder. `cairn backup`
writes a consistent copy while Cairn runs:

```bash
docker exec cairn /cairn backup /data/backup.db
docker cp cairn:/data/backup.db ./cairn-backup.db
```

To restore it, stop Cairn, copy it into the volume with a throwaway container (the Cairn image has no shell), and
start Cairn again:

```bash
docker compose stop cairn
docker run --rm --volumes-from cairn -v "$PWD/cairn-backup.db:/backup.db:ro" alpine:3.22 sh -c 'rm -f /data/cairn.db-wal /data/cairn.db-shm && cp /backup.db /data/cairn.db && chown 10001:10001 /data/cairn.db'
docker compose start cairn
```

On a new machine, run `docker compose up -d` once first, so the container and its volume exist.

### Upgrading

```bash
docker compose pull && docker compose up -d
```

The database is updated in place on start. Take a backup first.

### Fail2ban

Cairn answers `429` on its own after 5 failed sign-ins per e-mail or 10 per address in 15 minutes. Fail2ban adds a
ban for an address that tries many accounts. A failed password sign-in is the only `401` Cairn answers, logged as
`auth.login_failed` with the visitor's address; the filter counts that line and nothing else.

<!-- file: examples/compose/fail2ban/cairn.conf -->
```ini
# Fail2ban filter for Cairn: counts only failed password sign-ins (the 401 of POST /api/auth/login).
# Matches Cairn's JSON log line as written (journald, docker logs) and as Docker's json-file log stores it (inside
# "log", quotes escaped). Both patterns are anchored at the start of the line and spell out the fields before the
# address, so text someone typed (an e-mail, another app's log) can never supply the address that gets banned.
# Fail2ban cuts the date out of the line before matching, which is why the timestamp's value is "[^"]*".
# Copy to /etc/fail2ban/filter.d/cairn.conf.
[Definition]
failregex = ^\{"timestamp":"[^"]*","level":"WARN","event":"auth\.login_failed","ip":"<HOST>",
            ^\{"log":"\{\\"timestamp\\":\\"[^"\\]*\\",\\"level\\":\\"WARN\\",\\"event\\":\\"auth\.login_failed\\",\\"ip\\":\\"<HOST>\\",
ignoreregex =
```

<!-- file: examples/compose/fail2ban/jail.local -->
```ini
# Fail2ban jail for Cairn. Copy to /etc/fail2ban/jail.d/cairn.local.
# Cairn answers 429 itself after 5 failures per e-mail or 10 per address in 15 minutes; this bans an address
# that tries many accounts before that, and leaves a person who mistypes their own password to Cairn.
[cairn]
enabled  = true
filter   = cairn
# Docker's json-file logs; the filter matches only Cairn's line. Some distributions (Ubuntu 24.04) default to
# backend = systemd, which ignores logpath, so the backend is set here.
backend  = auto
logpath  = /var/lib/docker/containers/*/*-json.log
maxretry = 8
findtime = 15m
bantime  = 1h
# Published ports skip the INPUT chain, so the ban goes in Docker's DOCKER-USER chain, which only the iptables
# actions reach (Ubuntu 24.04 defaults to nftables, which uses a table of its own).
banaction = iptables-multiport
chain    = DOCKER-USER
port     = http,https
```

Keep `maxretry` between 6 and 10 and `findtime` at 15 minutes or more: a person mistyping their own password then
meets Cairn's `429` before any ban. Behind a proxy, set `CAIRN_TRUSTED_PROXIES` first, or the ban hits the proxy.
The jail sets `backend` and `banaction` itself, because some distributions (Ubuntu 24.04) default to the systemd
journal and to nftables, which would leave it reading nothing and banning nowhere Docker's ports pass. With the
journald log driver instead of json-file, use `backend = systemd` and `journalmatch = CONTAINER_NAME=cairn`.

## Building from source

You need Node 24 and Rust stable.

```bash
cd web && npm ci && npm run build && cd ..
cargo build --release
```

The front-end is built into `web/dist` and embedded in the binary, so build it first. The binary is
`target/release/cairn`.

## Contributing

Bug reports and pull requests are welcome. [`CONTRIBUTING.md`](CONTRIBUTING.md) has the checks a change must
pass and the rules the interface follows.

## License

Cairn is free software under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only). If you run a modified version for other people over a network, you must offer them its source.
