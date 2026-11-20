# eink-renderer

Show a **Home Assistant dashboard on an e-paper display**. Your real Lovelace cards and themes are rendered in a headless Chrome and served as a small black-and-white PNG that an ESPHome display fetches.

![The E-ink panel: live dashboard next to the image the display receives](docs/images/panel.png)

- **Set up like pairing a device.** Run one container, let Home Assistant discover it, pick a view and a display. No tokens, no YAML.
- **Preview in Home Assistant.** A sidebar panel shows the live dashboard next to the exact image the display gets.
- **Light and robust.** The image is re-rendered only when the dashboard changes, and the last good image keeps being served if anything restarts.

## What you need

- Home Assistant **2026.11 or newer**.
- A Linux machine with Docker, **x86-64 or 64-bit ARM** (a Raspberry Pi 4/5 with a 64-bit OS works), with about **220 MB of RAM** to spare.
- A display that fetches a PNG over HTTP, typically an ESP32 e-paper board running ESPHome. Black-and-white panels only.

## Setup

### 1. Run the renderer

```bash
docker run -d --name eink-renderer --restart unless-stopped --network host -v eink-data:/data ghcr.io/packetinspector/eink-renderer
```

That is the whole configuration. Until it is set up, it serves a "Ready to set up" picture.

`--network host` is what lets Home Assistant find it by itself. If you prefer `-p 10001:10001`, everything works the same, but you type the renderer's address in step 3.

### 2. Install the integration

- **With [HACS](https://hacs.xyz):** *HACS → ⋮ → Custom repositories*, add `https://github.com/packetinspector/eink-renderer` as an *Integration*, then download **E-ink Renderer**.
- **Without HACS:** copy `custom_components/eink_renderer/` into your Home Assistant `config/custom_components/`.

Restart Home Assistant once.

### 3. Add it in Home Assistant

Open *Settings → Devices & services*. The renderer is listed under **Discovered**; choose **Add**.

![Setup in Home Assistant: pick a view and a display](docs/images/setup-form.png)

1. Pick the dashboard view and your display, then submit.
2. Check the preview, then submit.
3. Copy the ESPHome snippet from the last dialog.

Not discovered? Choose **Add integration → E-ink Renderer** and enter the renderer's address, for example `192.168.0.50`.

### 4. Point the display at it

```yaml
http_request:
  timeout: 10s

online_image:
  - id: dashboard_image
    url: http://RENDERER_HOST:10001/eink.png
    format: PNG
    type: BINARY
    buffer_size: 10000
    update_interval: 60s
    on_download_finished:
      - if:
          condition:
            lambda: 'return !cached;'   # refresh the panel only when the picture changed
          then:
            - component.update: my_display

display:
  - platform: waveshare_epaper
    id: my_display
    model: 7.50inv2p                    # your panel, plus its pins
    update_interval: never
    lambda: |-
      it.image(0, 0, id(dashboard_image));
```

If the display shows white on black, switch off *Invert black and white* under **Configure**.

## What you get in Home Assistant

- **E-ink sidebar panel:** the live view beside the rendered image, with a shortcut to edit the view.
- **Configure:** change the view, display size, contrast, dithering or theme at any time. Changes apply at once, with a preview.
- **Entities:** the rendered image (usable in any card or notification), state, last render, memory, and a **problem** sensor you can use in automations.
- **Optional E-ink theme:** pure black on white with bold borders, in `ha/themes/eink.yaml`.

## Good to know

- **One view per container.** Run one container per display.
- **The image is not password-protected.** The display cannot log in, so anyone on your network can fetch the picture. Keep the renderer on your LAN.
- **No 32-bit ARM** (Pi Zero, or a 32-bit Raspberry Pi OS): Chrome is not built for it.
- **The integration has no icon yet** in the Home Assistant UI.

## Something not working?

| You see | Do this |
|---|---|
| Not discovered | Add it by address. Docker bridge networking, VLANs and some access points block discovery. |
| "Already paired" | Run `docker exec eink-renderer eink-renderer --reset-pairing`, then restart the container. |
| "The renderer can't show the dashboard yet" | The renderer cannot reach Home Assistant at that address. Correct it in the dialog. |
| A custom card shows an error, or text looks thin | See [troubleshooting](docs/troubleshooting.md). |

The problem sensor's `message` attribute and `docker logs eink-renderer` say what is wrong.

## More

- [Configuration reference](docs/configuration.md): running without the integration, every setting, the HTTP endpoints, the E-ink theme.
- [Troubleshooting](docs/troubleshooting.md)
- [How it works](docs/internals.md): rendering, memory use, pairing and security, pinned versions, development.
- [Changelog](CHANGELOG.md)

**Image:** `ghcr.io/packetinspector/eink-renderer`, for x86-64 and 64-bit ARM. Tags: `latest`, or a version such as `0.3.1`.

**Licence:** [MIT](LICENSE)
