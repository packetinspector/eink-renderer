"""Fixtures."""

from __future__ import annotations

import time
from unittest.mock import AsyncMock, patch

import pytest

pytest_plugins = "pytest_homeassistant_custom_component"

HOST = "192.0.2.10"
PORT = 10001
BASE = f"http://{HOST}:{PORT}"
RID = "abc123def456"


@pytest.fixture(autouse=True)
def auto_enable_custom_integrations(enable_custom_integrations):
    yield


@pytest.fixture(autouse=True)
def no_waiting():
    with patch("custom_components.eink_renderer.config_flow.asyncio.sleep", AsyncMock()):
        yield


@pytest.fixture(autouse=True)
def no_frontend():
    """The panel needs the real frontend; the flows and entities do not."""
    with (
        patch("custom_components.eink_renderer.async_update_panel", AsyncMock()),
        patch("custom_components.eink_renderer.async_setup_http", AsyncMock()),
        patch("custom_components.eink_renderer.config_flow.async_setup_http", AsyncMock()),
        patch(
            "custom_components.eink_renderer.config_flow.async_list_views",
            AsyncMock(return_value=[("/lovelace/0", "Overview › Home"), ("/lovelace/kitchen", "Overview › Kitchen")]),
        ),
    ):
        yield


def info(paired: bool = False, persistent: bool = True, locked: list[str] | None = None) -> dict:
    return {
        "id": RID, "api": 1, "name": "E-ink Renderer", "version": "0.3.0", "configured": paired, "paired": paired,
        "paired_with": "Elsewhere" if paired else None, "paired_at": None, "persistent": persistent,
        "pairing_available": persistent and not paired, "locked": locked or [], "addresses": [HOST], "port": PORT,
    }


def status(problem: dict | None = None, frame_at: float | None = None) -> dict:
    now = time.time()
    return {
        "status": {"engine_state": "running", "engine_restarts": 0, "problem": problem,
                   "last_frame_at": frame_at if frame_at is not None else now + 3600, "last_render_at": now},
        "setup": info(paired=True) | {"paired_with": "Test"},
        "image": {"etag": "\"x\"", "bytes": 100, "rendered_at": now, "width": 800, "height": 480, "placeholder": False},
        "memory": {"container_pss_kb": 220000},
        "config": {"dashboard_path": "/lovelace/0", "width": 800, "height": 480, "timezone": "UTC", "locked": []},
    }
