"""Sensors: renderer state, last render, memory, restarts."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from homeassistant.components.sensor import (
    SensorDeviceClass,
    SensorEntity,
    SensorEntityDescription,
    SensorStateClass,
)
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import EntityCategory, UnitOfInformation
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .entity import EinkEntity

STATES = ["setup", "init", "starting", "loading", "running", "restarting"]


@dataclass(frozen=True, kw_only=True)
class EinkSensorDescription(SensorEntityDescription):
    value: Callable[[dict[str, Any]], Any]


def _ts(v: Any) -> datetime | None:
    return datetime.fromtimestamp(v, UTC) if v else None


SENSORS: tuple[EinkSensorDescription, ...] = (
    EinkSensorDescription(
        key="state",
        translation_key="state",
        device_class=SensorDeviceClass.ENUM,
        options=STATES,
        value=lambda d: (d.get("status") or {}).get("engine_state") if (d.get("status") or {}).get("engine_state") in STATES else None,
    ),
    EinkSensorDescription(
        key="last_render",
        translation_key="last_render",
        device_class=SensorDeviceClass.TIMESTAMP,
        value=lambda d: _ts((d.get("status") or {}).get("last_render_at")),
    ),
    EinkSensorDescription(
        key="memory",
        translation_key="memory",
        device_class=SensorDeviceClass.DATA_SIZE,
        native_unit_of_measurement=UnitOfInformation.MEGABYTES,
        state_class=SensorStateClass.MEASUREMENT,
        suggested_display_precision=0,
        entity_category=EntityCategory.DIAGNOSTIC,
        value=lambda d: round(((d.get("memory") or {}).get("container_pss_kb") or 0) / 1024, 1) or None,
    ),
    EinkSensorDescription(
        key="restarts",
        translation_key="restarts",
        state_class=SensorStateClass.TOTAL_INCREASING,
        entity_category=EntityCategory.DIAGNOSTIC,
        value=lambda d: (d.get("status") or {}).get("engine_restarts"),
    ),
)


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddConfigEntryEntitiesCallback) -> None:
    async_add_entities(EinkSensor(entry.runtime_data, d) for d in SENSORS)


class EinkSensor(EinkEntity, SensorEntity):
    entity_description: EinkSensorDescription

    def __init__(self, coordinator, description: EinkSensorDescription) -> None:
        super().__init__(coordinator, description.key)
        self.entity_description = description

    @property
    def native_value(self) -> Any:
        return self.entity_description.value(self.coordinator.data or {})
