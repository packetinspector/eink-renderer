# Setup experience: design

Goal: setting up a display should feel like pairing a device. Run one container; Home Assistant finds it and asks two questions a person can answer by looking; done. No tokens, no YAML, no restarts beyond the one HA needs to load any custom integration.

## The path a new user takes

1. **Run the renderer.** `docker run -d --name eink-renderer --network host -v eink-data:/data <image>`. No environment variables. An unconfigured renderer:
   - serves a placeholder image saying *"Ready to set up — Home Assistant → Settings → Devices & services"* (so a display that is already flashed shows instructions, not an error);
   - shows the same on its status page `http://host:10001/`;
   - announces itself over mDNS as `_eink-renderer._tcp` with its id, version and paired state.
2. **Install the integration** (HACS → *E-ink Renderer* → Download, restart HA once). This restart is required by Home Assistant for any custom integration; it disappears if the integration ever becomes a core integration.
3. **Home Assistant shows "Discovered: E-ink Renderer"** under Settings → Devices & services. Click *Add*.
   - No discovery (renderer on Docker bridge networking, other VLAN, mDNS blocked)? *Add integration → E-ink Renderer* asks for one thing: the renderer's address (`192.168.0.50` or `192.168.0.50:10001`).
4. **"What should the display show?"** One dropdown listing this Home Assistant's real dashboards and views ("Overview › Kitchen"). Default: the first view of the default dashboard.
5. **"Which display?"** (on the same form as step 4) One dropdown of presets ("Seeed XIAO 7.5″ ePaper — 800×480, black/white" preselected; Waveshare 7.5″/4.2″/2.9″; Custom size). Threshold, inversion, dithering and theme get sensible defaults and live in *Configure* afterwards.
6. **Preview.** The flow pairs, waits for the first render and shows the actual 1-bit image plus the ESPHome `online_image` snippet with the right URL. *Submit* finishes.

Result, without YAML or restarts: a device *E-ink Renderer* with entities (image, state, last render, memory, problem), an **E-ink** sidebar panel served by the integration (live view next to the rendered image, both through Home Assistant, so it works over https and remotely), and the ESPHome snippet in the panel for later.

Changing anything later: device page → *Configure* (view, display size, threshold, invert, dither, theme). Applied live (the renderer restarts its browser, not its container) and the options flow shows the new preview.

Step count, measured in a real browser on a fresh Home Assistant 2026.11 (after the integration is installed): discovery path **1 command, 0 fields typed, 6 clicks** (Settings → Devices & services → Add → Submit → Submit preview → Finish; view and display were one form, as built), 1 dropdown pick only if you want another view than the default. By address: **1 command, 2 typed fields** (search box, address), 8 clicks.

## Pairing and the claim decision

- The integration creates a dedicated Home Assistant user **"E-ink Renderer"** in the **read-only** group, local-network only, with no password (it cannot log in interactively), and a long-lived access token for it. The user is visible under Settings → People → Users and is removed (token revoked) when the integration entry is deleted.
- The integration sends the renderer `POST /api/pair` with the HA URL (Home Assistant's own internal URL, as the renderer must reach it), the token and the display settings. The renderer stores them in `/data/state.json` (mode 600) and answers with a random **API key** that only the integration knows. From then on every change (`/api/settings`, `/api/unpair`) needs that key; the image and status endpoints stay unauthenticated read-only (the display must fetch without credentials).
- **Claim protection: trust on first use, but never silent.** An unpaired renderer accepts the first pairing. A pairing code would cost every user a typed field to defend against an attacker who must already be on the LAN, find the renderer in the minutes between `docker run` and setup, and gains only control over what an e-paper display shows (an unpaired renderer holds no secrets). What matters is that a hijack cannot go unnoticed and cannot happen later:
  - once paired, the renderer refuses `/api/pair` with *"already paired with <name> since <time>"*; the config flow shows exactly that with the reset command, so a stolen claim is visible at the moment you try to set it up;
  - re-pairing requires local access to the container: `docker exec eink-renderer eink-renderer --reset-pairing` (or deleting the volume);
  - the renderer's status page and log state who it is paired with.
- Headless/GitOps setups keep working: `HA_URL` + `HA_TOKEN(_FILE)` + other env vars (or `config.toml`) configure the renderer without pairing; env wins over the file, the file wins over what Home Assistant set. Such a renderer can still be added to Home Assistant for entities and the panel; settings locked by env/file are shown as such in *Configure*.

## Network notes

- mDNS needs the renderer on the same L2 network as Home Assistant: `--network host` (Linux). Docker's default bridge network does not pass multicast, so with `-p 10001:10001` discovery does not happen and the one-field manual path is used instead. The renderer advertises only addresses of physical-looking interfaces (not `docker0`, `br-*`, `veth*`, Tailscale) unless `ADVERTISE_IP` is set.
- The display fetches `http://<renderer>:10001/eink.png` directly (ESP32 → renderer), never through Home Assistant: it keeps working when HA restarts and needs no credentials on the device.

## What Home Assistant does not allow, and what was built instead

- **Images inside a config/options flow form**: possible. Descriptions are Markdown, and Markdown images cannot send auth headers, so the integration serves the preview at an unguessable, short-lived (15 min) URL scoped to the flow and shows it in the step description.
- **Creating the credential**: possible with Home Assistant's auth manager (`async_create_user` in the read-only group + a long-lived refresh token). It is the same mechanism the frontend uses for long-lived tokens; no deprecated API.
- **Restart-free install of a custom integration**: not possible; one restart after downloading via HACS.
