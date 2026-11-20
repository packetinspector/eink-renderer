# Changelog

All notable changes. Versions follow [Semantic Versioning](https://semver.org/).

## [0.3.1] - 2026-11-20

Pre-publish hardening and fixes from a fresh-user pass.

### Security
- Public `/status` and `/api/info` no longer reveal the Home Assistant URL, dashboard, problem messages, who the renderer is paired with or its addresses; the full status is `GET /api/status` with the pairing key. `/healthz` no longer echoes problem messages.
- Two simultaneous first-pairing requests can no longer both succeed; pairing logs the requesting address.
- Unless Home Assistant itself is on loopback, the rendered page cannot reach loopback, `0.0.0.0` or link-local/metadata addresses.
- At most 64 concurrent connections; `/data` is mode 700.

### Fixed
- A dashboard view deleted after setup is reported (problem sensor) within seconds and the last good image is kept, instead of the display silently showing another view.
- A renderer that lost its data volume: Home Assistant offers one re-pair prompt (no duplicate "Discovered" card) and re-pairing adopts its new id; the prompt also appears when the renderer was reset while Home Assistant was down.
- Home Assistant time-zone changes reach the renderer without a restart.
- Setup flow: typed addresses are validated; "nothing answering" vs "something else answering"; messages no longer mention `HA_URL` to integration users; strings pass hassfest.
- Panel halves fit side by side at 1900 px.

### CI
- Actions pinned to commit SHAs; hassfest, HACS validation and integration tests added; the image is pushed only for `v*` tags.

## [0.3.0] - 2026-11-20

Setup without configuration, and a Home Assistant integration.

### Added
- **Zero-configuration start:** without `HA_URL`/token the renderer waits to be set up, serves a "Ready to set up" image and status page, and announces itself over mDNS (`_eink-renderer._tcp`) on the host's LAN addresses.
- **Pairing API:** `GET /api/info`, `POST /api/pair` (first claim only), `POST /api/settings` and `POST /api/unpair` (API key). State in `/data/state.json` (mode 600); `--reset-pairing` CLI.
- Settings changed by Home Assistant are applied live (browser session restart, no container restart).
- Page time zone setting (`Emulation.setTimezoneOverride`); the integration sends Home Assistant's zone.
- **Home Assistant integration** (`custom_components/eink_renderer`, HACS-compatible): zeroconf discovery or one-field manual add; one setup form (dashboard/view dropdown + display preset); automatic pairing with a dedicated read-only, local-only user and token; preview of the first image; ESPHome snippet; image, state, last render, memory, restarts and problem entities; sidebar panel served by the integration (no cache-buster to manage, works over https); options flow applied live with preview; re-pairing via reauth; diagnostics.
- Example `docker-compose.yaml` (host network, discovery) and `docker-compose.headless.yaml`.

### Changed
- Precedence: defaults < Home Assistant settings < config file < environment; locked keys are reported.
- `CACHE_FILE` defaults to `/data/last.png` when `/data` exists.
- On a Docker bridge network the renderer no longer shows or announces its internal 172.17–31 address.

### Removed
- The YAML sidebar panel (`ha/eink_panel.yaml`, `ha/www/eink-panel/`), replaced by the integration's panel.

### Fixed (sidebar panel, before the integration)
- The live view no longer depends on the renderer; warnings at the top; single element registration; rebuild on re-attach.

## [0.2.0] - 2026-11-20

First version intended for other people.

### Added
- Configuration through environment variables (`HA_URL`, `HA_TOKEN` / `HA_TOKEN_FILE`, `DASHBOARD_PATH`, `WIDTH`, `HEIGHT`, `THRESHOLD`, `INVERT`, `DITHER`, `THEME`, `FONT_HINTING`, `PORT`, …); the TOML file is optional and environment variables take precedence.
- Start-up diagnostics: unreachable Home Assistant, rejected token, unknown dashboard, unknown view. Each is reported in the log, in `/status` (`problem`) and as a generated placeholder image until the first good frame.
- Frames are not published while Home Assistant is disconnected or still starting.
- linux/arm64 image (Chrome for Testing `linux-arm64`, checksummed); Rust is cross-compiled on the build host.
- `tini` as init in the image, `HEALTHCHECK` built in, `--version` / `--help`.
- Optional Home Assistant extras: E-ink theme, sidebar preview panel with a configurable renderer address.
- Unit tests, CI workflow (fmt, clippy, tests, multi-arch build, GHCR publish on tag).

### Changed
- `--font-render-hinting` is now a setting (`FONT_HINTING`, default `slight`).
- `block_resources` defaults to empty.
- Default timezone is UTC (set `TZ`).

### Removed
- The optional `process_madvise` page-out of the browser's file pages, together with the custom seccomp profile and `CAP_SYS_NICE` it needed. The kernel reclaims those clean pages on its own under memory pressure.

## [0.1.0] - 2026-11-20

Initial private version: supervised single-process chrome-headless-shell, repaint-driven capture via CDP screencast, 1-bit PNG with ETag/304, last-good image across browser restarts.
