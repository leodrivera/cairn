# Contributing to Cairn

Cairn aims to run well on a low-end PC: efficiency comes before features. A change that adds a dependency, a
background task or state about files needs a good reason, and the budget below must hold.

## Building and testing

You need Node 24, Rust stable and Docker. CI runs:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
(cd web && npm ci && npm run check && npm test && npm run build)
docker build -t cairn:ci .
scripts/check-budget.sh cairn:ci     # binary under 12 MB, idle memory under 20 MB, front-end under 100 kB gzipped
scripts/check-image.sh cairn:ci      # what the docs promise about the image
scripts/check-licenses.sh            # every shipped dependency under a license AGPL-3.0 accepts
```

A change to the image, the Compose examples or the install guide also runs the guide against the image:

```bash
tests/compose/run.sh cairn:ci
```

Behavior is described in [`docs/architecture.md`](docs/architecture.md); when the code and that document disagree,
one of them has a bug. Write the test first and watch it fail. Test data uses `example.com` addresses and English
names, with odd characters where they matter (`Report #1 (50%) ✓ café.txt`).

## Interface rules

### Look and motion

The Rust backend is small, so the interface is too.

- **Flat surfaces.** No blur, shadows or gradients. Depth is a one-step change of surface color and a 1 px line.
- **No web fonts.** The UI uses the system sans and mono fonts. The wordmark is an SVG.
- **Thumbnails on demand.** Gallery view loads previews lazily and only for what is on screen. List view loads none.
- **Still by default.** No animation beyond a 120 ms fade, and none under reduced motion. Long lists are windowed.

### Brand

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

### Voice

Plain, quiet, exact.

- "Moved 3 items to Archive." not "Success! Your files have been moved!"
- "Upload failed: no space left on /srv/data." not "Oops, something went wrong."

## License of contributions

Cairn is under AGPL-3.0-only. There is no CLA: by sending a contribution you license it under AGPL-3.0-only, like
the rest of the code.
