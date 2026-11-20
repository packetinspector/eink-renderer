"""E-ink Renderer: Home Assistant dashboards on e-paper displays."""

from __future__ import annotations

import logging

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import EVENT_CORE_CONFIG_UPDATE, Platform
from homeassistant.core import Event, HomeAssistant, callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession

from .api import RendererClient, RendererError
from .const import CONF_API_KEY, CONF_HOST, CONF_PORT, CONF_USER_ID, DOMAIN
from .coordinator import EinkCoordinator
from .credentials import async_remove_credentials
from .panel import async_setup_http, async_update_panel

_LOGGER = logging.getLogger(__name__)
PLATFORMS = [Platform.BINARY_SENSOR, Platform.IMAGE, Platform.SENSOR]

type EinkConfigEntry = ConfigEntry[EinkCoordinator]


async def async_setup_entry(hass: HomeAssistant, entry: EinkConfigEntry) -> bool:
    client = RendererClient(async_get_clientsession(hass), entry.data[CONF_HOST], entry.data[CONF_PORT], entry.data.get(CONF_API_KEY))
    coordinator = EinkCoordinator(hass, entry, client)
    # Not first_refresh: when the renderer is down at startup the entities still appear (problem sensor on).
    await coordinator.async_refresh()
    entry.runtime_data = coordinator
    await _sync_timezone(hass, coordinator)
    await async_setup_http(hass)
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    await async_update_panel(hass)

    @callback
    def _check_pairing() -> None:
        p = coordinator.problem
        if p and p.get("kind") == "unpaired":
            entry.async_start_reauth(hass)

    entry.async_on_unload(coordinator.async_add_listener(_check_pairing))
    _check_pairing()  # also when the renderer was already reset before Home Assistant started
    entry.async_on_unload(entry.add_update_listener(_options_updated))

    async def _core_config_changed(event: Event) -> None:
        # Home Assistant's time zone changed: keep the renderer's clock cards in step without a restart
        await coordinator.async_refresh()
        await _sync_timezone(hass, coordinator)

    entry.async_on_unload(hass.bus.async_listen(EVENT_CORE_CONFIG_UPDATE, _core_config_changed))
    return True


async def _sync_timezone(hass: HomeAssistant, coordinator: EinkCoordinator) -> None:
    """Keep the renderer's page time zone equal to Home Assistant's (clock cards), unless TZ is fixed there."""
    cfg = (coordinator.data or {}).get("config") or {}
    if not cfg or "timezone" in cfg.get("locked", []) or cfg.get("timezone") == hass.config.time_zone:
        return
    try:
        await coordinator.client.update_settings({"timezone": hass.config.time_zone})
    except Exception as err:  # noqa: BLE001 - never block setup over the time zone
        _LOGGER.debug("Could not set the renderer time zone: %s", err)


async def _options_updated(hass: HomeAssistant, entry: EinkConfigEntry) -> None:
    # settings were already sent to the renderer by the options flow; refresh state and the panel config
    await entry.runtime_data.async_request_refresh()
    await async_update_panel(hass)


async def async_unload_entry(hass: HomeAssistant, entry: EinkConfigEntry) -> bool:
    ok = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if ok:
        await async_update_panel(hass, exclude_entry_id=entry.entry_id)
    return ok


async def async_remove_entry(hass: HomeAssistant, entry: ConfigEntry) -> None:
    """Entry deleted: release the renderer and delete the user (revokes its token)."""
    client = RendererClient(async_get_clientsession(hass), entry.data[CONF_HOST], entry.data[CONF_PORT], entry.data.get(CONF_API_KEY))
    try:
        await client.unpair()
    except RendererError as err:
        _LOGGER.warning("Could not unpair %s: %s", client.base_url, err.message)
    await async_remove_credentials(hass, entry.data.get(CONF_USER_ID))
