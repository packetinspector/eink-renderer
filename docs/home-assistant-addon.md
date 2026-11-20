# Design note: packaging eink-renderer as a Home Assistant add-on

Not implemented. Since 0.3.0 the **integration** already gives discovery, automatic pairing, entities and a panel through Home Assistant. An add-on would additionally remove the separate container for HA OS / Supervised users (install from an add-on repository instead of running Docker). This is what that wrapper would need.

## What the add-on gains

- **No token handling.** Add-ons with `homeassistant_api: true` get `SUPERVISOR_TOKEN` and can reach Core at `http://supervisor/core`. The frontend, however, must be loaded from Core's own HTTP server, and `hassTokens` needs a real access token. Two options:
  1. keep asking the user for a long-lived token in the add-on options (simple, works today);
  2. set `HA_URL=http://homeassistant:8123` (the internal hostname add-ons can reach) and mint the token from the user's own long-lived token entered once. The Supervisor token is not accepted by the frontend websocket as a user token, so option 1 is the realistic one.
- **Ingress for the preview.** With `ingress: true` the status page and images are served through HA's authenticated ingress proxy (same origin, https if HA is https). That removes the panel's mixed-content problem and the need to know the renderer's address: the sidebar entry becomes the add-on's ingress panel (`panel_icon`, `panel_title`), and the `panel_custom` YAML step disappears. The service would need to honour the `X-Ingress-Path` header for its links (it only uses relative URLs today, so this is small).
- **The display still needs a plain port.** ESPHome fetches `/eink.png` without HA auth, so the add-on also exposes `ports: {10001/tcp: 10001}` for the device (ingress is auth-only and cannot be used by the panel's firmware).

## Files

```
eink-renderer/            # add-on folder in an add-on repository
  config.yaml             # name, slug, version, arch: [amd64, aarch64], ports, ingress, options/schema
  Dockerfile              # FROM ghcr.io/packetinspector/eink-renderer:<version> (prebuilt image, no build on the device)
  run.sh                  # read /data/options.json -> export HA_URL, HA_TOKEN, DASHBOARD_PATH, ... -> exec eink-renderer
  translations/en.yaml    # option labels
  DOCS.md
```

`config.yaml` options/schema map one-to-one to the environment variables (`ha_url` default `http://homeassistant:8123`, `token: password`, `dashboard_path: str`, `width/height: int`, `threshold: int(0,255)`, `invert: bool`, `dither: list(none|atkinson|floyd-steinberg)`, `theme: str?`, `block_resources: [str]?`). `CACHE_FILE=/data/last.png` gives persistence for free. `image: ghcr.io/packetinspector/eink-renderer-addon-{arch}` with `arch: [amd64, aarch64]` matches the two architectures the container already supports; armv7 (Pi 2/3 32-bit) is not possible because Chrome for Testing has no 32-bit ARM build.

## Open questions

- Memory on small HA hosts (Pi 3/4 with 1-2 GB): about 80 MB anonymous plus about 140 MB of reclaimable binary pages. Fine on 2 GB+, tight on 1 GB.
- The Supervisor's AppArmor profile must allow Chrome's `--no-sandbox` single-process mode; needs testing on HA OS.
- Watchdog: `watchdog: http://[HOST]:[PORT:10001]/healthz` maps directly onto the existing health endpoint.
