"""Constants for the E-ink Renderer integration."""

from __future__ import annotations

from datetime import timedelta
from typing import Final

DOMAIN: Final = "eink_renderer"
DEFAULT_PORT: Final = 10001
UPDATE_INTERVAL: Final = timedelta(seconds=15)

# config entry data
CONF_HOST: Final = "host"
CONF_PORT: Final = "port"
CONF_API_KEY: Final = "api_key"
CONF_RENDERER_ID: Final = "renderer_id"
CONF_USER_ID: Final = "user_id"
CONF_HA_URL: Final = "ha_url"

# options (mirror the renderer's settings)
OPT_DASHBOARD_PATH: Final = "dashboard_path"
OPT_DISPLAY: Final = "display"
OPT_WIDTH: Final = "width"
OPT_HEIGHT: Final = "height"
OPT_THRESHOLD: Final = "threshold"
OPT_INVERT: Final = "invert"
OPT_DITHER: Final = "dither"
OPT_THEME: Final = "theme"

DITHER_MODES: Final = ["none", "atkinson", "floyd-steinberg"]
DEFAULTS: Final = {OPT_THRESHOLD: 220, OPT_INVERT: True, OPT_DITHER: "none", OPT_THEME: ""}

CUSTOM_DISPLAY: Final = "custom"
# key -> (label, width, height, invert)
DISPLAYS: Final[dict[str, tuple[str, int, int, bool]]] = {
    "seeed_xiao_75": ('Seeed XIAO 7.5" ePaper panel (800×480, black/white)', 800, 480, True),
    "waveshare_75_v2": ('Waveshare 7.5" e-Paper V2 (800×480, black/white)', 800, 480, True),
    "waveshare_42": ('Waveshare 4.2" e-Paper (400×300, black/white)', 400, 300, True),
    "waveshare_29": ('Waveshare 2.9" e-Paper (296×128, black/white)', 296, 128, True),
}
DEFAULT_DISPLAY: Final = "seeed_xiao_75"

PANEL_URL_PATH: Final = "e-ink"
PANEL_ELEMENT: Final = "eink-renderer-panel"
PANEL_TITLE: Final = "E-ink"
PANEL_ICON: Final = "mdi:monitor-shimmer"
STATIC_URL: Final = "/eink_renderer_static"

USER_NAME: Final = "E-ink Renderer"
