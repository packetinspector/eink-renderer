# Configuration reference

With the Home Assistant integration you need none of this: settings are made in Home Assistant and stored on the renderer in `/data/state.json`. This page is for running without the integration, and for the settings the dialogs do not show.

## Running without the integration

```bash
docker run -d --name eink-renderer -p 10001:10001 -v eink-data:/data \
  -e HA_URL=http://homeassistant.local:8123 \
  -e HA_TOKEN_FILE=/run/secrets/ha_token -v ./ha_token:/run/secrets/ha_token:ro \
  -e DASHBOARD_PATH=/lovelace/0 -e TZ=Europe/Berlin \
  ghcr.io/packetinspector/eink-renderer
```

The token is a long-lived access token from your Home Assistant profile page. `docker-compose.headless.yaml` has the same setup as a compose file.

You can still add such a renderer to Home Assistant for the entities and the panel. Settings fixed by a file or the environment are shown there as locked.

## Settings

Every setting is an environment variable. A TOML file is optional: mount it at `/etc/eink-renderer/config.toml` or set `CONFIG_FILE`. Its keys are the lower-case variable names; see `config.example.toml`.

**Precedence:** built-in defaults < settings made in Home Assistant < config file < environment variables.

| Variable | Default | Meaning |
|---|---|---|
| `HA_URL` | – | Home Assistant's address as seen from the container. Without it and a token, the renderer waits to be set up from Home Assistant. |
| `HA_TOKEN` / `HA_TOKEN_FILE` | – | Long-lived access token, or a file containing it (preferred). Never logged. |
| `DASHBOARD_PATH` | `/lovelace/0` | Dashboard and view, by index (`/lovelace/0`) or by view path (`/my-dashboard/my-view`). |
| `WIDTH`, `HEIGHT` | `800`, `480` | Output size in pixels. |
| `HEADER_CROP` | `56` | Height of the Home Assistant toolbar cropped off the top. |
| `THRESHOLD` | `220` | With `DITHER=none`: grey at or above this becomes white. High values keep light-grey text readable. |
| `INVERT` | `true` | Invert the final image. Right for the common Waveshare 7.5" setups. |
| `DITHER` | `none` | `none`, `atkinson` or `floyd-steinberg`. Dithering suits photos, not small text. |
| `THEME` | Home Assistant's default | Theme for the render, for example `E-ink`. A view's own `theme:` wins. |
| `DARK` | `false` | Use the theme's dark mode. |
| `FONT_HINTING` | `slight` | `none`, `slight`, `medium` or `full`. `full` gives the crispest text with hinted fonts. |
| `TZ` | Home Assistant's zone, or `UTC` without the integration | Time zone of the rendered page. Clock cards use it. |
| `BLOCK_RESOURCES` | empty | Optional memory saver, see below. |
| `PORT` / `LISTEN` | `10001` / `0.0.0.0:10001` | Listen port or address. |
| `DATA_DIR` | `/data` | Pairing state and the last image. |
| `CACHE_FILE` | `$DATA_DIR/last.png` | The last image, served immediately after a restart. |
| `NAME` | `E-ink Renderer` | Name announced on the network and shown in Home Assistant. |
| `ADVERTISE` / `ADVERTISE_IP` | `true` / automatic | Network announcement on or off, and the address to announce. |
| `STALE_AFTER_SECS` | `300` | No repaint for this long reloads the page; a second time restarts the browser. |
| `CHROME_FLAGS` | empty | Extra browser flags, space-separated. |

### `BLOCK_RESOURCES`

Home Assistant loads every custom-card bundle you have installed on every dashboard, including ones the e-ink view never uses. List URL fragments here and the renderer's browser receives an empty module instead:

```
BLOCK_RESOURCES=/hacsfiles/,/local/community/
```

This saved about 18 MB on a setup with several chart and map cards. Leave out any card your e-ink view uses, or it renders as an error card.

## Endpoints

| Path | What |
|---|---|
| `/eink.png` | The image for the display. Supports `ETag` and `304 Not Modified`. |
| `/preview.png` | The same image without inversion, for people. |
| `/grey.png` | The frame before black-and-white conversion, for tuning `THRESHOLD` or a theme. |
| `/healthz` | `200 ok` when a real frame exists and nothing is wrong, otherwise `503` with the kind of problem. |
| `/status` | JSON: engine state, restarts, last render, memory. No addresses or secrets. |
| `/` | Small status page. |
| `GET /api/info` | Id, version, and whether it is set up and paired. Used by discovery. |
| `POST /api/pair` | First pairing only. |
| `GET /api/status`, `POST /api/settings`, `POST /api/unpair` | Need the pairing key that only your Home Assistant holds. |

The image and status endpoints are read-only and need no login, because the display must fetch without credentials.

## What the display needs

ESPHome's `online_image` needs an exact `Content-Length`, a PNG the same size as its buffer, and a standard colour type. The renderer sends a non-interlaced, 1-bit palette PNG (black, white).

ESPHome fires `on_download_finished` even when nothing changed, with `cached = true`. The `!cached` condition in the README snippet skips the panel refresh in that case. Without it the panel refreshes on every poll.

## E-ink theme

`ha/themes/eink.yaml` is an optional theme made for 1-bit output: pure black on white, no shadows or greys, 2 px card borders, black icons.

1. Copy the file to `<config>/themes/`. Home Assistant needs `frontend: themes: !include_dir_merge_named themes` in its configuration.
2. Reload themes.
3. Pick it under **Configure**, or set `theme: E-ink` on the view.

It pairs well with `FONT_HINTING=full`.
