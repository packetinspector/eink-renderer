"""Small async client for the renderer's HTTP API."""

from __future__ import annotations

import ipaddress
import re
from typing import Any

import aiohttp

TIMEOUT = aiohttp.ClientTimeout(total=10, connect=5)
_HOST_RE = re.compile(r"^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$")


class RendererError(Exception):
    """Error from the renderer (or not reachable)."""

    def __init__(self, kind: str, message: str, status: int | None = None, data: dict | None = None) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.status = status
        self.data = data or {}


def parse_address(text: str, default_port: int) -> tuple[str, int]:
    """Accept '1.2.3.4', '1.2.3.4:10001', 'http://host:10001/', 'host'."""
    t = text.strip()
    for prefix in ("http://", "https://"):
        if t.lower().startswith(prefix):
            t = t[len(prefix) :]
    t = t.split("/", 1)[0]
    if not t:
        raise ValueError("empty address")
    if t.startswith("["):  # [ipv6]:port
        host, _, rest = t[1:].partition("]")
        port = int(rest[1:]) if rest.startswith(":") else default_port
        ipaddress.IPv6Address(host)
    elif t.count(":") == 1:
        host, port_s = t.split(":")
        port = int(port_s)
    else:
        host, port = t, default_port
    if not t.startswith("[") and not _HOST_RE.match(host):
        raise ValueError(f"not a host name or address: {host!r}")
    if not 0 < port < 65536:
        raise ValueError(f"port out of range: {port}")
    return host, port


class RendererClient:
    """Talks to one renderer."""

    def __init__(self, session: aiohttp.ClientSession, host: str, port: int, api_key: str | None = None) -> None:
        self._session = session
        self.host = host
        self.port = port
        self.api_key = api_key

    @property
    def base_url(self) -> str:
        host = f"[{self.host}]" if ":" in self.host else self.host
        return f"http://{host}:{self.port}"

    async def _request(self, method: str, path: str, json: Any = None, auth: bool = False) -> dict[str, Any]:
        headers = {"Authorization": f"Bearer {self.api_key}"} if auth and self.api_key else {}
        try:
            async with self._session.request(
                method, self.base_url + path, json=json, headers=headers, timeout=TIMEOUT
            ) as resp:
                try:
                    data = await resp.json(content_type=None)
                except (aiohttp.ContentTypeError, ValueError):
                    data = {}
                if resp.status >= 400:
                    raise RendererError(
                        data.get("error", "http_error"),
                        data.get("message", f"HTTP {resp.status}"),
                        resp.status,
                        data,
                    )
                return data
        except RendererError:
            raise
        except (aiohttp.ClientConnectorError, TimeoutError, OSError) as err:
            raise RendererError(
                "cannot_connect", f"The renderer at {self.host}:{self.port} is not answering. Is its container running?"
            ) from err
        except aiohttp.ClientError as err:
            raise RendererError(
                "not_a_renderer",
                f"Something answered at {self.host}:{self.port}, but it is not an eink-renderer (wrong port?).",
            ) from err

    async def info(self) -> dict[str, Any]:
        """GET /api/info (no auth)."""
        data = await self._request("GET", "/api/info")
        if "id" not in data or data.get("api") != 1:
            raise RendererError("not_a_renderer", f"{self.base_url} does not look like an eink-renderer")
        return data

    async def status(self) -> dict[str, Any]:
        """GET /status (no auth): minimal public status."""
        return await self._request("GET", "/status")

    async def full_status(self) -> dict[str, Any]:
        """GET /api/status (API key): status with problem messages and the effective configuration.

        Falls back to the public /status on renderers older than 0.3.1 (404)."""
        try:
            return await self._request("GET", "/api/status", auth=True)
        except RendererError as err:
            if err.status == 404:
                return await self.status()
            raise

    async def pair(self, ha_name: str, settings: dict[str, Any]) -> dict[str, Any]:
        """POST /api/pair. Returns the renderer info including `api_key`."""
        return await self._request("POST", "/api/pair", json={"ha_name": ha_name, "settings": settings})

    async def update_settings(self, settings: dict[str, Any]) -> dict[str, Any]:
        """POST /api/settings (needs the API key)."""
        return await self._request("POST", "/api/settings", json=settings, auth=True)

    async def unpair(self) -> dict[str, Any]:
        """POST /api/unpair (needs the API key)."""
        return await self._request("POST", "/api/unpair", json={}, auth=True)

    async def image(self, variant: str = "preview") -> bytes:
        """The rendered image: 'preview' (normal polarity) or 'eink' (exact bytes the display gets)."""
        path = "/eink.png" if variant == "eink" else "/preview.png"
        try:
            async with self._session.get(self.base_url + path, timeout=TIMEOUT) as resp:
                if resp.status != 200:
                    raise RendererError("http_error", f"HTTP {resp.status}", resp.status)
                return await resp.read()
        except RendererError:
            raise
        except (aiohttp.ClientError, TimeoutError, OSError) as err:
            raise RendererError(
                "cannot_connect", f"The renderer at {self.host}:{self.port} is not answering. Is its container running?"
            ) from err

    def esphome_snippet(self) -> str:
        """The `online_image` block for an ESPHome device."""
        return (
            "online_image:\n"
            "  - id: dashboard_image\n"
            f"    url: {self.base_url}/eink.png\n"
            "    format: PNG\n"
            "    type: BINARY\n"
            "    buffer_size: 10000\n"
            "    update_interval: 60s\n"
            "    on_download_finished:\n"
            "      - if:\n"
            "          condition:\n"
            "            lambda: 'return !cached;'\n"
            "          then:\n"
            "            - component.update: my_display\n"
        )
