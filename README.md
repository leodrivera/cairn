<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/cairn-logo-on-dark.svg">
    <img src="brand/cairn-logo-on-light.svg" height="48" alt="cairn">
  </picture>
</h1>

<p align="center"><strong>Your files. One small binary.</strong></p>

Cairn is a self-hosted web file browser written in Rust. Point it at a folder on your server and browse, upload, edit and share its contents from any device. It is built to stay out of the way: small enough to run comfortably on an old PC.

> **Status:** early design phase. Nothing below is implemented yet; this README describes the intended scope.

## Why

A cairn is a stack of stones that marks a trail. Cairn stacks your folders and files where you can find them again.

## Planned features

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
- English interface with a small translation table; Português (Brasil) is planned first
- Single static binary, no runtime to install, shipped as a small Docker image

## Non-goals

These keep Cairn small. Each one needs extra modules or state that a light server should not carry.

- No terminal or command runner
- No trash: deletes are permanent, behind a confirmation
- No recent or starred files: Cairn keeps no state about your files, so nothing goes stale when you move them outside it
- No video preview, and no server-side image processing

## Design principles

The Rust backend is small, so the interface is too.

- **Flat surfaces.** No blur, shadows or gradients. Depth is a one-step change of surface color and a 1 px line.
- **No web fonts.** The UI uses the system sans and mono fonts. The wordmark is an SVG.
- **Thumbnails on demand.** Gallery view loads previews lazily and only for what is on screen. List view loads none.
- **Still by default.** No animation beyond a 120 ms fade, and none under reduced motion. Long lists are windowed.

## Brand

| Role | Name | Hex |
| --- | --- | --- |
| Dark ground | Basalt | `#141513` |
| Dark surface | Slate | `#1B1C1A` |
| Dark raised | Granite | `#252622` |
| Dark text | Chalk | `#ECEBE6` |
| Light ground | Paper | `#F4F3EE` |
| Accent on dark | Lichen | `#C5D86D` |
| Accent on light | Moss | `#4A6600` |
| Destructive on dark | Ember | `#F0906B` |
| Info on dark | Tarn | `#86B9DB` |

Brand type is Bricolage Grotesque 800, used for the wordmark only, and shipped as SVG outlines. All tokens are in [`tokens.css`](tokens.css).

Logo and icon files are in [`brand/`](brand): `cairn-logo.svg` (mark and wordmark), `cairn-wordmark.svg`, `cairn-mark.svg`, `favicon.svg`, `favicon.ico`, `apple-touch-icon.png`, `icon-192.png` and `icon-512.png`. Those SVGs use `currentColor` and follow the text color of the page when inlined. For use as a plain image, `cairn-logo-on-light.svg` and `cairn-logo-on-dark.svg` have fixed colors.

```html
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
```

## Voice

Plain, quiet, exact.

- "Moved 3 items to Archive." not "Success! Your files have been moved!"
- "Upload failed: no space left on /srv/data." not "Oops, something went wrong."

## Installation

Cairn will ship as a Docker image. Mount the folder to browse at `/srv` and a folder for the database at `/data`, and configure it with environment variables. The planned variables and Compose examples are in [`docs/architecture.md`](docs/architecture.md).

[TODO: tested instructions once there is a release]

## Building from source

You need Node 24 and Rust stable.

```bash
cd web && npm ci && npm run build && cd ..
cargo build --release
```

The front-end is built into `web/dist` and embedded in the binary, so build it first. The binary is
`target/release/cairn`.

## License

Cairn is free software under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only). If you run a modified version for other people over a network, you must offer them its source.
