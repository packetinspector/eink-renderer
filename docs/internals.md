# How it works

## Rendering

1. A small Rust supervisor starts a pinned `chrome-headless-shell` and talks to it over the DevTools pipe. No debugging port is opened.
2. It signs the browser in to Home Assistant with a read-only token and opens the chosen view at the display's size.
3. While the page loads it diagnoses problems: Home Assistant unreachable, token rejected, unknown dashboard or view. Problems appear in the log, in the problem sensor and, until the first good frame, as the image itself.
4. Chrome sends a frame only when the page repaints. Frames are skipped while Home Assistant is disconnected or starting, so a "Home Assistant is starting" screen never reaches the display.
5. Each frame is cropped, converted to grey, thresholded or dithered, optionally inverted, and encoded as a 1-bit PNG. If the bytes changed, it becomes the served image with a new `ETag`.
6. A health check every 30 s, a stale-repaint check and the browser's exit status trigger a page reload or a browser restart. The last good image is served throughout.
7. Settings changed in Home Assistant restart the browser session (about a second), not the container.

## Memory

Measured with a five-card dashboard on Chrome 158:

| | Total | Of which real memory |
|---|---:|---:|
| x86-64 | about 235 MB | about 90 MB |
| x86-64 with `BLOCK_RESOURCES` for unused custom cards | about 220 MB | about 80 MB |
| 64-bit ARM (Raspberry Pi) | about 237 MB | about 95 MB |

The rest, about 135–140 MB, is Chrome's program code. The kernel can drop it when memory is tight and reread it from disk, which is why the container's own memory figure is much lower. A `mem_limit` of 300–400 MB is a safe ceiling.

Between updates the renderer uses about 1 % of one CPU core.

## Pairing and security

- **A dedicated, read-only user.** The integration creates a Home Assistant user called "E-ink Renderer": read-only, local network only, no password. It is deleted, with its token, when you delete the integration. The renderer only ever holds that token.
- **First pairing wins.** The renderer accepts one pairing and refuses later attempts. All changes need a key that only your Home Assistant holds. Someone on your network could only claim a renderer nobody has set up yet, and you would see it at once, because adding it would fail with "already paired". There is deliberately no pairing code to type.
- **The browser cannot reach the renderer host's own services.** Requests from the rendered page to loopback and link-local addresses are refused, unless Home Assistant itself is at such an address.
- **Chrome runs without its own sandbox**, because a locked-down container does not have the privileges it needs. The container is the isolation boundary: non-root, no capabilities, read-only root filesystem in the example compose file. Point it only at your own Home Assistant.
- **The image and basic status need no login** and are served over plain HTTP.

## What is pinned

| | Version |
|---|---|
| Browser | Chrome for Testing `chrome-headless-shell` 158.0.8127.36, verified by checksum per architecture |
| Rust toolchain | `rust:1.100.0-trixie` |
| Base image | `debian:trixie-slim` |
| Rust crates | `Cargo.lock`, built with `--locked` |

To update Chrome, take a version and both checksums from the [Chrome for Testing dashboard](https://googlechromelabs.github.io/chrome-for-testing/), update `CHROME_VERSION`, `CHROME_SHA256_AMD64` and `CHROME_SHA256_ARM64` in the `Dockerfile`, rebuild, and compare `/eink.png` with the previous build. There is no automatic update.

## What has been tested

- The published 0.3.1 image was pulled and started on x86-64 and on a Raspberry Pi (64-bit ARM).
- The first-time setup, by discovery and by address, was driven in a real browser against a fresh Home Assistant 2026.11.
- The integration's automated tests run against Home Assistant 2026.10.4, the newest test harness available at the time.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The service itself runs on Linux only.

Integration tests (Python 3.14):

```bash
pip install -r requirements_test.txt home-assistant-frontend && pytest
```

## Releases

`.github/workflows/release.yml` runs the checks and builds the image for both architectures on every push. A `v*` tag publishes it to `ghcr.io/packetinspector/eink-renderer`.

Design notes: [setup experience](setup-experience.md), [add-on sketch](home-assistant-addon.md).
