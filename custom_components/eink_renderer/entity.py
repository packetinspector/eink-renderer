"""Base entity."""

from __future__ import annotations

from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from .const import CONF_RENDERER_ID, DOMAIN
from .coordinator import EinkCoordinator


class EinkEntity(CoordinatorEntity[EinkCoordinator]):
    """An entity of one renderer device."""

    _attr_has_entity_name = True

    def __init__(self, coordinator: EinkCoordinator, key: str) -> None:
        super().__init__(coordinator)
        entry = coordinator.config_entry
        rid = entry.data[CONF_RENDERER_ID]
        self._attr_unique_id = f"{rid}_{key}"
        sw = ((coordinator.data or {}).get("setup") or {}).get("version")
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, rid)},
            name=entry.title,
            manufacturer="eink-renderer",
            model="Dashboard renderer",
            sw_version=sw,
            configuration_url=coordinator.client.base_url + "/",
        )
