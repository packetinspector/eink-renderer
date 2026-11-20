"""Sidebar panel + HTTP views. The browser only ever talks to Home Assistant, never to the renderer."""

from __future__ import annotations

import hashlib
import secrets
import time
from pathlib import Path
from typing import Any

from aiohttp import web

from homeassistant.components import frontend, panel_custom
from homeassistant.components.http import KEY_HASS, HomeAssistantView, StaticPathConfig
from homeassistant.config_entries import ConfigEntryState
from homeassistant.core import HomeAssistant
from homeassistant.helpers import entity_registry as er
from homeassistant.helpers.aiohttp_client import async_get_clientsession

from .api import RendererClient, RendererError
from .const import (
    CONF_HOST,
    CONF_PORT,
    CONF_RENDERER_ID,
    DOMAIN,
    OPT_DASHBOARD_PATH,
    PANEL_ELEMENT,
    PANEL_ICON,
    PANEL_TITLE,
    PANEL_URL_PATH,
    STATIC_URL,
)

PANEL_JS = Path(__file__).parent / "frontend" / "eink-renderer-panel.js"
PREVIEW_TTL = 900


def _data(hass: HomeAssistant) -> dict[str, Any]:
    return hass.data.setdefault(DOMAIN, {"previews": {}, "views": False, "static": False, "panel": False})


async def async_setup_http(hass: HomeAssistant) -> None:
    """Register views and the static panel file once per Home Assistant run."""
    d = _data(hass)
    if not d["views"]:
        hass.http.register_view(EinkImageView())
        hass.http.register_view(EinkPreviewView())
        d["views"] = True
    if not d["static"]:
        await hass.http.async_register_static_paths(
            [StaticPathConfig(f"{STATIC_URL}/eink-renderer-panel.js", str(PANEL_JS), cache_headers=True)]
        )
        d["static"] = True


def _panel_version() -> str:
    return hashlib.sha256(PANEL_JS.read_bytes()).hexdigest()[:12]


async def async_update_panel(hass: HomeAssistant, exclude_entry_id: str | None = None) -> None:
    """(Re-)register the sidebar panel with the current entries, or remove it when there are none.

    Called from async_setup_entry (entry still SETUP_IN_PROGRESS) and on unload (pass exclude_entry_id).
    """
    d = _data(hass)
    ent_reg = er.async_get(hass)
    entries = []
    for entry in hass.config_entries.async_entries(DOMAIN):
        if entry.entry_id == exclude_entry_id or getattr(entry, "runtime_data", None) is None:
            continue
        if entry.state not in (ConfigEntryState.LOADED, ConfigEntryState.SETUP_IN_PROGRESS):
            continue
        rid = entry.data[CONF_RENDERER_ID]

        def eid(domain: str, key: str, rid: str = rid) -> str | None:
            return ent_reg.async_get_entity_id(domain, DOMAIN, f"{rid}_{key}")

        client: RendererClient = entry.runtime_data.client
        entries.append(
            {
                "entry_id": entry.entry_id,
                "title": entry.title,
                "dashboard_path": entry.options.get(OPT_DASHBOARD_PATH, "/lovelace/0"),
                "width": entry.options.get("width", 800),
                "height": entry.options.get("height", 480),
                "image_entity": eid("image", "image"),
                "problem_entity": eid("binary_sensor", "problem"),
                "state_entity": eid("sensor", "state"),
                "last_render_entity": eid("sensor", "last_render"),
                "memory_entity": eid("sensor", "memory"),
                "renderer_url": client.base_url,
                "esphome": client.esphome_snippet(),
            }
        )
    if d["panel"]:
        frontend.async_remove_panel(hass, PANEL_URL_PATH, warn_if_unknown=False)
        d["panel"] = False
    if not entries:
        return
    module_url = await hass.async_add_executor_job(_panel_version)
    await panel_custom.async_register_panel(
        hass,
        frontend_url_path=PANEL_URL_PATH,
        webcomponent_name=PANEL_ELEMENT,
        sidebar_title=PANEL_TITLE,
        sidebar_icon=PANEL_ICON,
        module_url=f"{STATIC_URL}/eink-renderer-panel.js?v={module_url}",
        config={"entries": entries},
        require_admin=False,
    )
    d["panel"] = True


def create_preview_url(hass: HomeAssistant, host: str, port: int) -> str:
    """Short-lived, unguessable URL that shows a renderer's current image inside a setup step."""
    d = _data(hass)
    now = time.monotonic()
    for tok in [t for t, v in d["previews"].items() if v[2] < now]:
        d["previews"].pop(tok, None)
    tok = secrets.token_urlsafe(24)
    d["previews"][tok] = (host, port, now + PREVIEW_TTL)
    return f"/api/{DOMAIN}/preview/{tok}?t={int(time.time())}"


class EinkPreviewView(HomeAssistantView):
    """Image for config/options flow descriptions (markdown images cannot send auth headers)."""

    url = f"/api/{DOMAIN}/preview/{{token}}"
    name = f"api:{DOMAIN}:preview"
    requires_auth = False

    async def get(self, request: web.Request, token: str) -> web.Response:
        hass: HomeAssistant = request.app[KEY_HASS]
        item = _data(hass)["previews"].get(token)
        if item is None or item[2] < time.monotonic():
            return web.Response(status=404)
        client = RendererClient(async_get_clientsession(hass), item[0], item[1])
        try:
            body = await client.image("preview")
        except RendererError:
            return web.Response(status=502)
        return web.Response(body=body, content_type="image/png", headers={"Cache-Control": "no-store"})


class EinkImageView(HomeAssistantView):
    """Authenticated image for the panel: ?variant=preview|eink."""

    url = f"/api/{DOMAIN}/{{entry_id}}/image"
    name = f"api:{DOMAIN}:image"
    requires_auth = True

    async def get(self, request: web.Request, entry_id: str) -> web.Response:
        hass: HomeAssistant = request.app[KEY_HASS]
        entry = hass.config_entries.async_get_entry(entry_id)
        if entry is None or entry.domain != DOMAIN or entry.state is not ConfigEntryState.LOADED:
            return web.Response(status=404)
        variant = request.query.get("variant", "preview")
        try:
            body = await entry.runtime_data.client.image("eink" if variant == "eink" else "preview")
        except RendererError as err:
            return web.Response(status=502, text=err.message)
        return web.Response(body=body, content_type="image/png", headers={"Cache-Control": "no-store"})


def client_for(hass: HomeAssistant, data: dict[str, Any], api_key: str | None = None) -> RendererClient:
    return RendererClient(async_get_clientsession(hass), data[CONF_HOST], data[CONF_PORT], api_key)
