"""Config and options flow tests."""

from __future__ import annotations

from ipaddress import ip_address

from homeassistant import config_entries
from homeassistant.auth.const import GROUP_ID_READ_ONLY
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo
from pytest_homeassistant_custom_component.common import MockConfigEntry
from pytest_homeassistant_custom_component.test_util.aiohttp import AiohttpClientMocker

from custom_components.eink_renderer.const import DOMAIN

from .conftest import BASE, HOST, PORT, RID, info, status


async def _through_progress(hass: HomeAssistant, flow_mgr, result):
    """Wait for the render step; the next configure returns the step after it."""
    while result["type"] in (FlowResultType.SHOW_PROGRESS, FlowResultType.SHOW_PROGRESS_DONE):
        await hass.async_block_till_done()
        result = await flow_mgr.async_configure(result["flow_id"])
    return result


async def test_manual_flow_pairs_and_creates_entry(hass: HomeAssistant, hass_owner_user, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/info", json=info())
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    aioclient_mock.post(f"{BASE}/api/pair", json=info(paired=True) | {"api_key": "secret-key"})

    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "user"

    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"address": f"http://{HOST}:{PORT}/"})
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "setup"

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"dashboard_path": "/lovelace/kitchen", "display": "waveshare_42"}
    )
    result = await _through_progress(hass, hass.config_entries.flow, result)
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "preview"
    assert "/api/eink_renderer/preview/" in result["description_placeholders"]["preview"]

    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"]["api_key"] == "secret-key"
    assert result["data"]["renderer_id"] == RID
    assert result["options"]["dashboard_path"] == "/lovelace/kitchen"
    assert (result["options"]["width"], result["options"]["height"]) == (400, 300)
    assert "online_image" in result["description_placeholders"]["esphome"]

    # the pairing request carried a token of a new read-only, local-only user
    pair_call = next(c for c in aioclient_mock.mock_calls if str(c[1]).endswith("/api/pair"))
    settings = pair_call[2]["settings"]
    assert settings["dashboard_path"] == "/lovelace/kitchen" and settings["token"] and settings["ha_url"].startswith("http")
    assert settings["timezone"] == hass.config.time_zone
    user = await hass.auth.async_get_user(result["data"]["user_id"])
    assert user is not None and user.local_only and [g.id for g in user.groups] == [GROUP_ID_READ_ONLY]
    assert not user.is_admin and not user.credentials and not user.is_owner


async def test_manual_flow_reports_already_paired(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/info", json=info(paired=True))
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"address": HOST})
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"base": "already_paired"}


async def test_manual_flow_cannot_connect(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/info", exc=TimeoutError())
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"address": f"{HOST}:{PORT}"})
    assert result["errors"] == {"base": "cannot_connect"}


def _zc(paired: str = "0") -> ZeroconfServiceInfo:
    return ZeroconfServiceInfo(
        ip_address=ip_address(HOST), ip_addresses=[ip_address(HOST)], port=PORT, hostname=f"eink-renderer-{RID}.local.",
        type="_eink-renderer._tcp.local.", name=f"E-ink Renderer {RID[:6]}._eink-renderer._tcp.local.",
        properties={"id": RID, "paired": paired, "version": "0.3.0", "api": "1"},
    )


async def test_zeroconf_goes_straight_to_setup(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/info", json=info())
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_ZEROCONF}, data=_zc())
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "setup"
    assert result["description_placeholders"]["host"] == f"{HOST}:{PORT}"


async def test_zeroconf_paired_elsewhere_aborts(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/info", json=info(paired=True))
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_ZEROCONF}, data=_zc("1"))
    assert result["type"] is FlowResultType.ABORT and result["reason"] == "already_paired"


async def test_zeroconf_known_renderer_updates_address(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    entry = MockConfigEntry(domain=DOMAIN, unique_id=RID, data={"host": "192.0.2.99", "port": PORT, "api_key": "k", "renderer_id": RID})
    entry.add_to_hass(hass)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_ZEROCONF}, data=_zc("1"))
    assert result["type"] is FlowResultType.ABORT and result["reason"] == "already_configured"
    assert entry.data["host"] == HOST


async def test_options_flow_applies_live_and_previews(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    aioclient_mock.get(f"{BASE}/api/status", json=status())
    aioclient_mock.post(f"{BASE}/api/settings", json={"config": {}})
    entry = MockConfigEntry(
        domain=DOMAIN, unique_id=RID, title="E-ink Renderer",
        data={"host": HOST, "port": PORT, "api_key": "k", "renderer_id": RID, "user_id": None},
        options={"dashboard_path": "/lovelace/0", "display": "seeed_xiao_75", "width": 800, "height": 480,
                 "threshold": 220, "invert": True, "dither": "none", "theme": ""},
    )
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()

    result = await hass.config_entries.options.async_init(entry.entry_id)
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "init"
    result = await hass.config_entries.options.async_configure(
        result["flow_id"],
        {"dashboard_path": "/lovelace/kitchen", "display": "seeed_xiao_75", "width": 640, "height": 384,
         "threshold": 180, "invert": False, "dither": "atkinson", "theme": "default"},
    )
    result = await _through_progress(hass, hass.config_entries.options, result)
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "preview"
    sent = [c for c in aioclient_mock.mock_calls if str(c[1]).endswith("/api/settings")][-1]
    assert sent[2] == {"dashboard_path": "/lovelace/kitchen", "width": 800, "height": 480, "threshold": 180,
                       "invert": False, "dither": "atkinson", "theme": "", "timezone": hass.config.time_zone}
    assert sent[3]["Authorization"] == "Bearer k"
    result = await hass.config_entries.options.async_configure(result["flow_id"], {})
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert entry.options["dashboard_path"] == "/lovelace/kitchen" and entry.options["threshold"] == 180


async def test_zeroconf_same_address_as_existing_entry(hass: HomeAssistant, aioclient_mock: AiohttpClientMocker) -> None:
    """A reset renderer announces a new id at the same address: no second 'Discovered' card."""
    entry = MockConfigEntry(domain=DOMAIN, unique_id="oldid", data={"host": HOST, "port": PORT, "api_key": "k", "renderer_id": "oldid"})
    entry.add_to_hass(hass)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_ZEROCONF}, data=_zc())
    assert result["type"] is FlowResultType.ABORT and result["reason"] == "already_configured"
