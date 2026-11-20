"""The rendered image as an image entity (served by Home Assistant: works over https, remotely, in cards)."""

from __future__ import annotations

from datetime import UTC, datetime

from homeassistant.components.image import ImageEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .api import RendererError
from .coordinator import EinkCoordinator
from .entity import EinkEntity


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddConfigEntryEntitiesCallback) -> None:
    async_add_entities([EinkImage(hass, entry.runtime_data)])


class EinkImage(EinkEntity, ImageEntity):
    """What the display shows (normal polarity)."""

    _attr_translation_key = "display"
    _attr_content_type = "image/png"

    def __init__(self, hass: HomeAssistant, coordinator: EinkCoordinator) -> None:
        EinkEntity.__init__(self, coordinator, "image")
        ImageEntity.__init__(self, hass)
        self._update_timestamp()

    def _update_timestamp(self) -> None:
        ts = ((self.coordinator.data or {}).get("status") or {}).get("last_render_at")
        self._attr_image_last_updated = datetime.fromtimestamp(ts, UTC) if ts else None

    @callback
    def _handle_coordinator_update(self) -> None:
        self._update_timestamp()
        super()._handle_coordinator_update()

    async def async_image(self) -> bytes | None:
        try:
            return await self.coordinator.client.image("preview")
        except RendererError:
            return None
