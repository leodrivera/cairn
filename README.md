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
- Share a file or folder with other people on the same server, view only or with editing, for a set time
- A trash: deleted items wait a set number of days (30 by default) and can be restored, or deleted for good
- Multiple users, each limited to a scope (a folder) with read, write, share and admin permissions
- Sign in with an e-mail and password, with OAuth providers through a broker such as Tinyauth, with any OpenID Connect provider directly, or any mix of these
- Optional CAPTCHA on the password form: reCAPTCHA v2, Cloudflare Turnstile or hCaptcha
- Light and dark themes, usable on desktop and phone
- The interface is in English and Português (Brasil), chosen in Profile
- Single static binary, no runtime to install, shipped as a small Docker image

## Non-goals

These keep Cairn small. Each one needs extra modules or state that a light server should not carry.

- No terminal or command runner
- No recent or starred files: Cairn keeps only grants and trash entries, by path, so nothing goes stale when you move files outside it
- No video preview, and no server-side image processing

## Installation

Cairn ships as a Docker image for x86_64 and aarch64 (64-bit ARM, such as a Raspberry Pi 4 or 5):
`ghcr.io/leodrivera/cairn`. Tags: `0.1.0` for one exact release, `0.1` for the newest `0.1.x` (the examples use this
one, so `docker compose pull` brings fixes), and `latest`.

The image holds only the binary and a list of certificate authorities. It runs as uid 10001. Mount the folder to
browse at `/srv` and a volume for the database at `/data`, and configure it with environment variables; every
variable is in [`docs/architecture.md`](docs/architecture.md#3-configuration).

The example below, and every one in [`docs/install.md`](docs/install.md), is a file in
[`examples/compose/`](examples/compose), and each one was started and checked against the image by
`tests/compose/run.sh` before this release.

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

### Upgrading

```bash
docker compose pull && docker compose up -d
```

The database is updated in place on start. [Take a backup](docs/install.md#backups-and-restore) first.

### More

[`docs/install.md`](docs/install.md) has the rest of the guide:

- [Behind Traefik, with HTTPS and reCAPTCHA](docs/install.md#behind-traefik-with-https-and-recaptcha), for a server
  reachable from the internet
- [Sign in through Tinyauth](docs/install.md#sign-in-through-tinyauth) or
  [with OpenID Connect](docs/install.md#sign-in-with-openid-connect), and
  [a provider with a certificate from your own CA](docs/install.md#a-provider-with-a-certificate-from-your-own-ca)
- [The trash](docs/install.md#the-trash), [backups and restore](docs/install.md#backups-and-restore) and
  [Fail2ban](docs/install.md#fail2ban)

## Building from source

You need Node 26 and Rust stable.

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
