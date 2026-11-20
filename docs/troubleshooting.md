# Troubleshooting

Start with the problem sensor's `message` attribute in Home Assistant, then `docker logs eink-renderer`.

## Setup

- **Not discovered.** The renderer must run with `--network host` on the same network segment as Home Assistant. Docker bridge networking, VLANs and some Wi-Fi access points block discovery. Add it by address instead: **Add integration → E-ink Renderer**.
- **"Already paired".** Something paired it first, perhaps an earlier Home Assistant. Run `docker exec eink-renderer eink-renderer --reset-pairing`, then restart the container.
- **"The renderer can't show the dashboard yet".** The renderer cannot reach Home Assistant at the address shown, for example a `.local` name the container cannot resolve. Correct it in that dialog.
- **The renderer lost its data volume.** Home Assistant offers to pair it again with one click.

## The picture

- **Clock shows the wrong time.** The integration sends Home Assistant's time zone. Without the integration, set `TZ`.
- **White text on black.** Switch off *Invert black and white* under **Configure**, or set `INVERT=false`.
- **Text looks ragged or thin.** Try the E-ink theme with `FONT_HINTING=full`, or lower the threshold. `/grey.png` shows the picture before it is converted.
- **A custom card shows an error.** Remove its prefix from `BLOCK_RESOURCES`.
- **Emoji, Chinese, Japanese or Korean text shows as boxes.** The image ships DejaVu fonts only. Build a derived image that installs more, for example `apt-get install fonts-noto-cjk`.
- **The panel's two halves differ slightly.** The live half is your own browser, with your theme and your browser's time zone. The rendered half always uses Home Assistant's time zone.

## Without the integration

- **"Access token rejected".** The token is wrong or was revoked. Create a new long-lived token.
- **"Cannot reach Home Assistant".** `HA_URL` must work from inside the container. `localhost` means the container itself; use the host's address, or `host.docker.internal` on Docker Desktop.
- **"Dashboard not found" or "View not found".** The message lists what Home Assistant reports. Views can be addressed by index (`/lovelace/2`) or by their `path`.
