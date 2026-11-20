"""Config flow: discover or type the address, pick the view and the display, preview, done."""

from __future__ import annotations

import asyncio
import logging
import time
from typing import Any
from urllib.parse import urlparse

import voluptuous as vol

from homeassistant.config_entries import ConfigEntry, ConfigFlow, ConfigFlowResult, OptionsFlow
from homeassistant.core import callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.selector import (
    BooleanSelector,
    NumberSelector,
    NumberSelectorConfig,
    NumberSelectorMode,
    SelectOptionDict,
    SelectSelector,
    SelectSelectorConfig,
    SelectSelectorMode,
    TextSelector,
    TextSelectorConfig,
    TextSelectorType,
    ThemeSelector,
    ThemeSelectorConfig,
)
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

from .api import RendererClient, RendererError, parse_address
from .const import (
    CONF_API_KEY,
    CONF_HA_URL,
    CONF_HOST,
    CONF_PORT,
    CONF_RENDERER_ID,
    CONF_USER_ID,
    CUSTOM_DISPLAY,
    DEFAULT_DISPLAY,
    DEFAULT_PORT,
    DEFAULTS,
    DISPLAYS,
    DITHER_MODES,
    DOMAIN,
    OPT_DASHBOARD_PATH,
    OPT_DISPLAY,
    OPT_DITHER,
    OPT_HEIGHT,
    OPT_INVERT,
    OPT_THEME,
    OPT_THRESHOLD,
    OPT_WIDTH,
)
from .credentials import async_create_credentials, async_new_token, async_remove_credentials
from .dashboards import async_list_views
from .panel import async_setup_http, create_preview_url

_LOGGER = logging.getLogger(__name__)
RENDER_TIMEOUT = 75


def display_options() -> list[SelectOptionDict]:
    opts = [SelectOptionDict(value=k, label=v[0]) for k, v in DISPLAYS.items()]
    opts.append(SelectOptionDict(value=CUSTOM_DISPLAY, label="Other display (custom size)"))
    return opts


def guess_display(width: int, height: int) -> str:
    for key, (_, w, h, _) in DISPLAYS.items():
        if (w, h) == (width, height):
            return key
    return CUSTOM_DISPLAY


def renderer_settings(options: dict[str, Any], timezone: str | None = None) -> dict[str, Any]:
    """Entry options -> renderer settings (plus Home Assistant's time zone, so clock cards match)."""
    extra = {"timezone": timezone} if timezone else {}
    return extra | {
        "dashboard_path": options[OPT_DASHBOARD_PATH],
        "width": int(options[OPT_WIDTH]),
        "height": int(options[OPT_HEIGHT]),
        "threshold": int(options.get(OPT_THRESHOLD, DEFAULTS[OPT_THRESHOLD])),
        "invert": bool(options.get(OPT_INVERT, DEFAULTS[OPT_INVERT])),
        "dither": options.get(OPT_DITHER, DEFAULTS[OPT_DITHER]),
        "theme": options.get(OPT_THEME, DEFAULTS[OPT_THEME]) or "",
    }


async def wait_for_frame(client: RendererClient, since: float, timeout: float = RENDER_TIMEOUT) -> dict[str, Any]:
    """Wait until the renderer delivered a frame newer than `since`, or reports a problem.

    While Home Assistant is still starting the renderer deliberately holds frames back (so a display never
    shows "Home Assistant is starting" placeholders); that counts as working after a short wait.
    """
    deadline = time.monotonic() + timeout
    started = time.monotonic()
    await asyncio.sleep(1.5)
    last = "no answer from the renderer"
    held_at_start: int | None = None
    while time.monotonic() < deadline:
        try:
            data = await (client.full_status() if client.api_key else client.status())
        except RendererError as err:
            last = err.message
        else:
            st = data.get("status") or {}
            if st.get("problem"):
                return {"ok": False, "problem": st["problem"]["message"]}
            img = data.get("image") or {}
            if img and not img.get("placeholder") and (st.get("last_frame_at") or 0) >= since - 1:
                return {"ok": True}
            held = st.get("frames_suppressed") or 0
            if held_at_start is None:
                held_at_start = held
            if st.get("engine_state") == "running" and held > held_at_start and time.monotonic() - started > 15:
                return {"ok": True, "note": "\n\nHome Assistant is still starting, so this is the last image from before; the display updates as soon as startup finishes."}
            last = f"renderer state: {st.get('engine_state')}"
        await asyncio.sleep(1)
    return {"ok": False, "problem": f"No image after {int(timeout)} seconds ({last})."}


def detect_ha_url(hass) -> str:
    """Home Assistant's address as the renderer (another host/container on the LAN) will reach it."""
    internal = hass.config.internal_url
    if internal:
        host = urlparse(internal).hostname or ""
        if not host.endswith(".local") and host not in ("localhost", "127.0.0.1"):
            return internal.rstrip("/")
    api = hass.config.api
    scheme = "https" if api and api.use_ssl else "http"
    ip = api.local_ip if api else "127.0.0.1"
    port = api.port if api else 8123
    return f"{scheme}://{ip}:{port}"


def locked_note(locked: list[str]) -> str:
    if not locked:
        return ""
    return (
        "\n\nThese settings are fixed by the renderer's environment variables or config file and "
        f"cannot be changed here: {', '.join(locked)}."
    )


class EinkConfigFlow(ConfigFlow, domain=DOMAIN):
    """Set up a renderer."""

    VERSION = 1

    def __init__(self) -> None:
        self._host: str | None = None
        self._port: int = DEFAULT_PORT
        self._info: dict[str, Any] = {}
        self._options: dict[str, Any] = {}
        self._view_label = ""
        self._user_id: str | None = None
        self._api_key: str | None = None
        self._ha_url: str | None = None
        self._task: asyncio.Task | None = None
        self._result: dict[str, Any] = {}
        self._finished = False
        self._preview_shown = False
        self._since = 0.0

    @staticmethod
    @callback
    def async_get_options_flow(config_entry: ConfigEntry) -> OptionsFlow:
        return EinkOptionsFlow()

    def _client(self, with_key: bool = False) -> RendererClient:
        return RendererClient(
            async_get_clientsession(self.hass), self._host, self._port, self._api_key if with_key else None
        )

    # --- finding the renderer -------------------------------------------------------------------

    async def async_step_zeroconf(self, discovery_info: ZeroconfServiceInfo) -> ConfigFlowResult:
        rid = discovery_info.properties.get("id")
        if not rid:
            return self.async_abort(reason="not_a_renderer")
        host = str(discovery_info.ip_address)
        port = discovery_info.port or DEFAULT_PORT
        # Same address as a renderer that is already set up (it was reset and got a new id): that entry's
        # re-pair prompt handles it, so don't offer it as a new device.
        for entry in self._async_current_entries(include_ignore=False):
            if entry.data.get(CONF_HOST) == host and entry.data.get(CONF_PORT) == port:
                return self.async_abort(reason="already_configured")
        await self.async_set_unique_id(rid)
        self._abort_if_unique_id_configured(updates={CONF_HOST: host, CONF_PORT: port})
        self._host, self._port = host, port
        try:
            self._info = await self._client().info()
        except RendererError:
            return self.async_abort(reason="cannot_connect")
        if self._info.get("paired"):
            return self.async_abort(reason="already_paired")
        self.context["title_placeholders"] = {"name": self._info.get("name", "E-ink Renderer"), "host": f"{host}:{port}"}
        return await self.async_step_setup()

    async def async_step_user(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        placeholders = {"error": ""}
        if user_input is not None:
            try:
                host, port = parse_address(user_input["address"], DEFAULT_PORT)
            except ValueError:
                errors["address"] = "invalid_address"
            else:
                self._host, self._port = host, port
                try:
                    self._info = await self._client().info()
                except RendererError as err:
                    errors["base"] = "cannot_connect" if err.kind == "cannot_connect" else "not_a_renderer"
                    placeholders["error"] = err.message
                else:
                    # an abandoned flow (closed tab) must not block a new manual attempt
                    await self.async_set_unique_id(self._info["id"], raise_on_progress=False)
                    self._abort_if_unique_id_configured(updates={CONF_HOST: host, CONF_PORT: port})
                    if self._info.get("paired"):
                        errors["base"] = "already_paired"
                    elif not self._info.get("persistent"):
                        errors["base"] = "no_storage"
                    else:
                        return await self.async_step_setup()
        schema = vol.Schema(
            {
                vol.Required("address", default=(user_input or {}).get("address", "")): TextSelector(
                    TextSelectorConfig(type=TextSelectorType.TEXT)
                )
            }
        )
        return self.async_show_form(step_id="user", data_schema=schema, errors=errors, description_placeholders=placeholders)

    # --- what to show, on which display ----------------------------------------------------------

    async def async_step_setup(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        views = await async_list_views(self.hass)
        cur = {"locked": self._info.get("locked", [])}
        if user_input is not None:
            self._view_label = dict(views).get(user_input[OPT_DASHBOARD_PATH], user_input[OPT_DASHBOARD_PATH])
            self._options = {OPT_DASHBOARD_PATH: user_input[OPT_DASHBOARD_PATH], OPT_DISPLAY: user_input[OPT_DISPLAY], **DEFAULTS}
            if user_input[OPT_DISPLAY] == CUSTOM_DISPLAY:
                return await self.async_step_custom_size()
            _, w, h, inv = DISPLAYS[user_input[OPT_DISPLAY]]
            self._options.update({OPT_WIDTH: w, OPT_HEIGHT: h})
            if "invert" not in cur.get("locked", []):
                self._options[OPT_INVERT] = inv
            return await self.async_step_pair()
        paths = [p for p, _ in views]
        default_view = paths[0] if paths else "/lovelace/0"
        default_display = DEFAULT_DISPLAY
        schema = vol.Schema(
            {
                vol.Required(OPT_DASHBOARD_PATH, default=default_view): SelectSelector(
                    SelectSelectorConfig(
                        options=[SelectOptionDict(value=p, label=label) for p, label in views] or [SelectOptionDict(value="/lovelace/0", label="Overview")],
                        mode=SelectSelectorMode.DROPDOWN,
                    )
                ),
                vol.Required(OPT_DISPLAY, default=default_display): SelectSelector(
                    SelectSelectorConfig(options=display_options(), mode=SelectSelectorMode.DROPDOWN)
                ),
            }
        )
        return self.async_show_form(
            step_id="setup",
            data_schema=schema,
            description_placeholders={
                "name": self._info.get("name", "E-ink Renderer"),
                "host": f"{self._host}:{self._port}",
                "locked": locked_note(cur.get("locked", [])),
            },
        )

    async def async_step_custom_size(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None:
            self._options.update({OPT_WIDTH: int(user_input[OPT_WIDTH]), OPT_HEIGHT: int(user_input[OPT_HEIGHT])})
            return await self.async_step_pair()
        size = NumberSelectorConfig(min=64, max=4096, step=1, mode=NumberSelectorMode.BOX, unit_of_measurement="px")
        schema = vol.Schema(
            {vol.Required(OPT_WIDTH, default=800): NumberSelector(size), vol.Required(OPT_HEIGHT, default=480): NumberSelector(size)}
        )
        return self.async_show_form(step_id="custom_size", data_schema=schema)

    # --- pairing ---------------------------------------------------------------------------------

    async def async_step_pair(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        await async_setup_http(self.hass)
        if self._ha_url is None:
            self._ha_url = detect_ha_url(self.hass)
        label = f"{self._host}:{self._port}"
        if self._user_id is None:
            self._user_id, token = await async_create_credentials(self.hass, label)
        else:
            token = await async_new_token(self.hass, self._user_id, label)
        settings = {"ha_url": self._ha_url, "token": token, **renderer_settings(self._options, self.hass.config.time_zone)}
        try:
            if self._api_key:
                await self._client(with_key=True).update_settings(settings)
            else:
                res = await self._client().pair(self.hass.config.location_name or "Home Assistant", settings)
                self._api_key = res["api_key"]
        except RendererError as err:
            if err.kind == "already_paired":
                return self.async_abort(reason="already_paired")
            if err.kind == "no_storage":
                return self.async_abort(reason="no_storage")
            return self.async_abort(reason="cannot_connect")
        self._since = time.time()
        return await self.async_step_render()

    async def async_step_render(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if self._task is None:
            self._task = self.hass.async_create_task(wait_for_frame(self._client(with_key=True), self._since))
        if not self._task.done():
            return self.async_show_progress(step_id="render", progress_action="render", progress_task=self._task)
        self._result = self._task.result()
        self._task = None
        return self.async_show_progress_done(next_step_id="preview" if self._result.get("ok") else "problem")

    async def async_step_problem(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None and user_input.get(CONF_HA_URL):
            self._ha_url = user_input[CONF_HA_URL].rstrip("/")
            self._preview_shown = False
            return await self.async_step_pair()
        schema = vol.Schema(
            {vol.Required(CONF_HA_URL, default=self._ha_url): TextSelector(TextSelectorConfig(type=TextSelectorType.URL))}
        )
        return self.async_show_form(
            step_id="problem", data_schema=schema, description_placeholders={"problem": self._result.get("problem", "")}
        )

    async def async_step_preview(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        client = self._client()
        # Only an explicit submit of the shown preview finishes (a progress step may call us with {}).
        if user_input is not None and self._preview_shown:
            self._finished = True
            return self.async_create_entry(
                title=self._info.get("name", "E-ink Renderer"),
                data={
                    CONF_HOST: self._host,
                    CONF_PORT: self._port,
                    CONF_API_KEY: self._api_key,
                    CONF_RENDERER_ID: self._info["id"],
                    CONF_USER_ID: self._user_id,
                    CONF_HA_URL: self._ha_url,
                },
                options=self._options,
                description="esphome",
                description_placeholders={"esphome": client.esphome_snippet(), "url": f"{client.base_url}/eink.png"},
            )
        self._preview_shown = True
        return self.async_show_form(
            step_id="preview",
            data_schema=vol.Schema({}),
            description_placeholders={
                "preview": create_preview_url(self.hass, self._host, self._port),
                "view": self._view_label,
                "url": f"{client.base_url}/eink.png",
                "size": f"{self._options[OPT_WIDTH]}×{self._options[OPT_HEIGHT]}",
                "note": self._result.get("note", ""),
            },
        )

    @callback
    def async_remove(self) -> None:
        """Flow abandoned before finishing: undo the pairing and delete the user we created."""
        if self._finished or self._user_id is None:
            return
        user_id, api_key = self._user_id, self._api_key
        client = RendererClient(async_get_clientsession(self.hass), self._host, self._port, api_key)

        async def cleanup() -> None:
            if api_key:
                try:
                    await client.unpair()
                except RendererError:
                    pass
            await async_remove_credentials(self.hass, user_id)

        self.hass.async_create_task(cleanup())

    # --- re-pairing (renderer lost its /data) ------------------------------------------------------

    async def async_step_reauth(self, entry_data: dict[str, Any]) -> ConfigFlowResult:
        return await self.async_step_reauth_confirm()

    async def async_step_reauth_confirm(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        entry = self._get_reauth_entry()
        if user_input is None:
            return self.async_show_form(step_id="reauth_confirm", description_placeholders={"name": entry.title})
        self._host, self._port = entry.data[CONF_HOST], entry.data[CONF_PORT]
        label = f"{self._host}:{self._port}"
        token = await async_new_token(self.hass, entry.data.get(CONF_USER_ID, ""), label)
        user_id = entry.data.get(CONF_USER_ID)
        if token is None:
            user_id, token = await async_create_credentials(self.hass, label)
        ha_url = entry.data.get(CONF_HA_URL) or detect_ha_url(self.hass)
        try:
            res = await self._client().pair(
                self.hass.config.location_name or "Home Assistant", {"ha_url": ha_url, "token": token, **renderer_settings(dict(entry.options), self.hass.config.time_zone)}
            )
        except RendererError as err:
            if err.kind == "already_paired":
                return self.async_abort(reason="already_paired")
            return self.async_abort(reason="cannot_connect")
        # the renderer may have a new id after a reset; keep entity ids (renderer_id) but match the new one
        return self.async_update_reload_and_abort(
            entry, unique_id=res.get("id") or entry.unique_id, data_updates={CONF_API_KEY: res["api_key"], CONF_USER_ID: user_id}
        )


class EinkOptionsFlow(OptionsFlow):
    """Change view, display and conversion later; applied live, with a preview."""

    def __init__(self) -> None:
        self._new: dict[str, Any] = {}
        self._task: asyncio.Task | None = None
        self._result: dict[str, Any] = {}
        self._since = 0.0
        self._pushed = False
        self._finished = False
        self._preview_shown = False

    @callback
    def async_remove(self) -> None:
        """Abandoned after the new settings were sent: put the saved settings back."""
        if not self._pushed or self._finished:
            return
        client = self._client()
        old = renderer_settings({**DEFAULTS, **self.config_entry.options}, self.hass.config.time_zone)

        async def revert() -> None:
            try:
                await client.update_settings(old)
            except RendererError:
                pass

        self.hass.async_create_task(revert())

    def _client(self) -> RendererClient:
        return self.config_entry.runtime_data.client

    async def async_step_init(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        opts = {**DEFAULTS, **self.config_entry.options}
        views = await async_list_views(self.hass)
        coordinator = self.config_entry.runtime_data
        locked = ((coordinator.data or {}).get("config") or {}).get("locked", [])
        errors: dict[str, str] = {}
        if user_input is not None:
            new = dict(user_input)
            theme = new.get(OPT_THEME) or ""
            new[OPT_THEME] = "" if theme in ("default", "none") else theme
            if new[OPT_DISPLAY] != CUSTOM_DISPLAY:
                _, w, h, _ = DISPLAYS[new[OPT_DISPLAY]]
                new[OPT_WIDTH], new[OPT_HEIGHT] = w, h
            new[OPT_WIDTH], new[OPT_HEIGHT], new[OPT_THRESHOLD] = int(new[OPT_WIDTH]), int(new[OPT_HEIGHT]), int(new[OPT_THRESHOLD])
            self._since = time.time()
            try:
                await self._client().update_settings(renderer_settings(new, self.hass.config.time_zone))
            except RendererError as err:
                errors["base"] = "cannot_connect" if err.kind == "cannot_connect" else "rejected"
                self._result = {"problem": err.message}
            else:
                self._new = new
                self._pushed = True
                return await self.async_step_render()
        paths = [p for p, _ in views]
        cur_path = opts.get(OPT_DASHBOARD_PATH, "/lovelace/0")
        view_options = [SelectOptionDict(value=p, label=label) for p, label in views]
        if cur_path not in paths:
            view_options.insert(0, SelectOptionDict(value=cur_path, label=cur_path))
        size = NumberSelectorConfig(min=64, max=4096, step=1, mode=NumberSelectorMode.BOX, unit_of_measurement="px")
        schema = vol.Schema(
            {
                vol.Required(OPT_DASHBOARD_PATH, default=cur_path): SelectSelector(
                    SelectSelectorConfig(options=view_options, mode=SelectSelectorMode.DROPDOWN)
                ),
                vol.Required(OPT_DISPLAY, default=opts.get(OPT_DISPLAY, guess_display(opts.get(OPT_WIDTH, 800), opts.get(OPT_HEIGHT, 480)))): SelectSelector(
                    SelectSelectorConfig(options=display_options(), mode=SelectSelectorMode.DROPDOWN)
                ),
                vol.Required(OPT_WIDTH, default=opts.get(OPT_WIDTH, 800)): NumberSelector(size),
                vol.Required(OPT_HEIGHT, default=opts.get(OPT_HEIGHT, 480)): NumberSelector(size),
                vol.Required(OPT_THRESHOLD, default=opts[OPT_THRESHOLD]): NumberSelector(
                    NumberSelectorConfig(min=1, max=255, step=1, mode=NumberSelectorMode.SLIDER)
                ),
                vol.Required(OPT_INVERT, default=opts[OPT_INVERT]): BooleanSelector(),
                vol.Required(OPT_DITHER, default=opts[OPT_DITHER]): SelectSelector(
                    SelectSelectorConfig(options=DITHER_MODES, mode=SelectSelectorMode.DROPDOWN, translation_key="dither")
                ),
                vol.Optional(OPT_THEME, default=opts[OPT_THEME] or "default"): ThemeSelector(ThemeSelectorConfig(include_default=True)),
            }
        )
        return self.async_show_form(
            step_id="init",
            data_schema=schema,
            errors=errors,
            description_placeholders={"locked": locked_note(locked), "error": self._result.get("problem", "")},
        )

    async def async_step_render(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if self._task is None:
            self._task = self.hass.async_create_task(wait_for_frame(self._client(), self._since))
        if not self._task.done():
            return self.async_show_progress(step_id="render", progress_action="render", progress_task=self._task)
        self._result = self._task.result()
        self._task = None
        return self.async_show_progress_done(next_step_id="preview")

    async def async_step_preview(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None and self._preview_shown:
            self._finished = True
            return self.async_create_entry(data=self._new)
        client = self._client()
        self._preview_shown = True
        status = self._result.get("note", "") if self._result.get("ok") else f"\n\n**The renderer reports a problem:** {self._result.get('problem', '')}"
        return self.async_show_form(
            step_id="preview",
            data_schema=vol.Schema({}),
            description_placeholders={"preview": create_preview_url(self.hass, client.host, client.port), "status": status},
        )
