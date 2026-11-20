"""Polls the renderer's /status."""

from __future__ import annotations

import logging
from typing import Any

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .api import RendererClient, RendererError
from .const import DOMAIN, UPDATE_INTERVAL

_LOGGER = logging.getLogger(__name__)


class EinkCoordinator(DataUpdateCoordinator[dict[str, Any]]):
    """Holds the latest /status of one renderer."""

    config_entry: ConfigEntry

    def __init__(self, hass: HomeAssistant, entry: ConfigEntry, client: RendererClient) -> None:
        super().__init__(hass, _LOGGER, config_entry=entry, name=DOMAIN, update_interval=UPDATE_INTERVAL)
        self.client = client
        self.last_error: str | None = None

    async def _async_update_data(self) -> dict[str, Any]:
        self.auth_problem: dict[str, Any] | None = None
        try:
            data = await self.client.full_status()
        except RendererError as err:
            if err.status != 401:
                self.last_error = err.message
                raise UpdateFailed(err.message) from err
            # The renderer no longer accepts our key: it lost its pairing, or was paired by someone else.
            try:
                data = await self.client.status()
            except RendererError as err2:
                self.last_error = err2.message
                raise UpdateFailed(err2.message) from err2
            if (data.get("setup") or {}).get("paired"):
                self.auth_problem = {
                    "kind": "paired_elsewhere",
                    "message": "The renderer is now paired with a different Home Assistant (or was reset and paired again). "
                    "Remove this integration entry, reset the renderer if needed, and add it again.",
                }
            else:
                self.auth_problem = {
                    "kind": "unpaired",
                    "message": "The renderer lost its pairing (was its /data volume reset?). Home Assistant offers to pair it again.",
                }
        self.last_error = None
        return data

    @property
    def problem(self) -> dict[str, Any] | None:
        """Current problem: renderer unreachable, or a problem the renderer reports."""
        if not self.last_update_success:
            return {"kind": "unreachable", "message": self.last_error or "The renderer is not reachable."}
        if getattr(self, "auth_problem", None):
            return self.auth_problem
        p = (self.data or {}).get("status", {}).get("problem")
        if p:
            return {"kind": p.get("kind"), "message": p.get("message") or f"Renderer problem: {p.get('kind')}"}
        return None
