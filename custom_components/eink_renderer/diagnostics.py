"""Diagnostics (secrets redacted)."""

from __future__ import annotations

from typing import Any

from homeassistant.components.diagnostics import async_redact_data
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant

from .const import CONF_API_KEY, CONF_USER_ID

TO_REDACT = {CONF_API_KEY, CONF_USER_ID, "token", "api_key"}


async def async_get_config_entry_diagnostics(hass: HomeAssistant, entry: ConfigEntry) -> dict[str, Any]:
    coordinator = entry.runtime_data
    return {
        "entry": {"data": async_redact_data(dict(entry.data), TO_REDACT), "options": dict(entry.options)},
        "renderer_status": async_redact_data(coordinator.data or {}, TO_REDACT),
        "last_update_success": coordinator.last_update_success,
        "problem": coordinator.problem,
        "esphome_snippet": coordinator.client.esphome_snippet(),
    }
