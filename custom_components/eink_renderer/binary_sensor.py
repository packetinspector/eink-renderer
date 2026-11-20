"""Problem sensor: on when the renderer is unreachable or reports a problem. Usable in automations."""

from __future__ import annotations

from typing import Any

from homeassistant.components.binary_sensor import BinarySensorDeviceClass, BinarySensorEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .entity import EinkEntity


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddConfigEntryEntitiesCallback) -> None:
    async_add_entities([EinkProblem(entry.runtime_data)])


class EinkProblem(EinkEntity, BinarySensorEntity):
    _attr_translation_key = "problem"
    _attr_device_class = BinarySensorDeviceClass.PROBLEM

    def __init__(self, coordinator) -> None:
        super().__init__(coordinator, "problem")

    @property
    def available(self) -> bool:
        # stays available while the renderer is down, so it can turn on
        return True

    @property
    def is_on(self) -> bool:
        return self.coordinator.problem is not None

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        p = self.coordinator.problem or {}
        return {"kind": p.get("kind"), "message": p.get("message")}
