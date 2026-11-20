"""Entity, problem sensor and removal tests."""

from __future__ import annotations

from homeassistant.core import HomeAssistant
from pytest_homeassistant_custom_component.common import MockConfigEntry
from pytest_homeassistant_custom_component.test_util.aiohttp import AiohttpClientMocker

from custom_components.eink_renderer.const import DOMAIN
from custom_components.eink_renderer.credentials import async_create_credentials

from .conftest import BASE, HOST, PORT, RID, status


def _entry(user_id: str | None = None) -> MockConfigEntry:
    return MockConfigEntry(
        domain=DOMAIN, unique_id=RID, title="E-ink Renderer",
        data={"host": HOST, "port": PORT, "api_key": "k", "renderer_id": RID, "user_id": user_id},
        options={"dashboard_path": "/lovelace/0", "width": 800, "height": 480},
    )


async def test_entities_when_running(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    aioclient_mock.post(f"{BASE}/api/settings", json={})
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert hass.states.get("binary_sensor.e_ink_renderer_problem").state == "off"
    assert hass.states.get("sensor.e_ink_renderer_state").state == "running"
    assert float(hass.states.get("sensor.e_ink_renderer_memory").state) == 214.8
    assert hass.states.get("image.e_ink_renderer_display") is not None
    assert hass.states.get("sensor.e_ink_renderer_last_render").state not in ("unknown", "unavailable")
    # the renderer reported UTC; Home Assistant's own zone is pushed so clock cards match
    tz_push = [c for c in aioclient_mock.mock_calls if str(c[1]).endswith("/api/settings")]
    assert tz_push and tz_push[0][2] == {"timezone": hass.config.time_zone} and hass.config.time_zone != "UTC"


async def test_problem_sensor_on_when_renderer_down(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", exc=TimeoutError())
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    problem = hass.states.get("binary_sensor.e_ink_renderer_problem")
    assert problem.state == "on"
    assert problem.attributes["kind"] == "unreachable"
    assert hass.states.get("sensor.e_ink_renderer_state").state == "unavailable"


async def test_problem_sensor_reports_renderer_problem(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", json=status(problem={"kind": "view", "message": "View '9' does not exist"}))
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    p = hass.states.get("binary_sensor.e_ink_renderer_problem")
    assert p.state == "on" and p.attributes["message"] == "View '9' does not exist"


async def test_remove_entry_unpairs_and_deletes_user(hass: HomeAssistant, hass_owner_user, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    aioclient_mock.post(f"{BASE}/api/unpair", json={})
    user_id, _token = await async_create_credentials(hass, "test")
    entry = _entry(user_id)
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    await hass.config_entries.async_remove(entry.entry_id)
    await hass.async_block_till_done()
    assert any(str(c[1]).endswith("/api/unpair") for c in aioclient_mock.mock_calls)
    assert await hass.auth.async_get_user(user_id) is None


async def test_renderer_lost_pairing_offers_repair(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    """Key rejected and the renderer says it is unpaired: problem 'unpaired' and a re-pair (reauth) flow."""
    aioclient_mock.get(f"{BASE}/api/status", status=401, json={"error": "unauthorized"})
    public = status()
    public["setup"]["paired"] = False
    public.pop("config")
    aioclient_mock.get(f"{BASE}/status", json=public)
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    p = hass.states.get("binary_sensor.e_ink_renderer_problem")
    assert p.state == "on" and p.attributes["kind"] == "unpaired"
    flows = hass.config_entries.flow.async_progress()
    assert any(f["context"]["source"] == "reauth" for f in flows)


async def test_renderer_paired_elsewhere(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", status=401, json={"error": "unauthorized"})
    aioclient_mock.get(f"{BASE}/status", json=status())
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    p = hass.states.get("binary_sensor.e_ink_renderer_problem")
    assert p.state == "on" and p.attributes["kind"] == "paired_elsewhere"
    assert not any(f["context"]["source"] == "reauth" for f in hass.config_entries.flow.async_progress())


async def test_old_renderer_without_api_status(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    """Renderers before 0.3.1 have no /api/status: fall back to /status."""
    aioclient_mock.get(f"{BASE}/api/status", status=404, json={"error": "not_found"})
    aioclient_mock.get(f"{BASE}/status", json=status())
    aioclient_mock.post(f"{BASE}/api/settings", json={})
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert hass.states.get("sensor.e_ink_renderer_state").state == "running"


async def test_address_parsing() -> None:
    from custom_components.eink_renderer.api import parse_address

    assert parse_address("192.0.2.5", 10001) == ("192.0.2.5", 10001)
    assert parse_address("http://renderer.lan:10007/", 10001) == ("renderer.lan", 10007)
    assert parse_address("[2001:db8::1]:10001", 1) == ("2001:db8::1", 10001)
    for bad in ("not a host!!", "host:0", "host:99999", "", "http://", "a_b:1"):
        try:
            parse_address(bad, 10001)
        except ValueError:
            continue
        raise AssertionError(f"accepted {bad!r}")


async def test_time_zone_follows_core_config(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    aioclient_mock.post(f"{BASE}/api/settings", json={})
    entry = _entry()
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    await hass.config.async_update(time_zone="Europe/Berlin")
    await hass.async_block_till_done()
    pushed = [c[2] for c in aioclient_mock.mock_calls if str(c[1]).endswith("/api/settings")]
    assert {"timezone": "Europe/Berlin"} in pushed
