"""List this Home Assistant's dashboards and views for the setup dropdown."""

from __future__ import annotations

import logging

from homeassistant.components.lovelace.const import LOVELACE_DATA
from homeassistant.core import HomeAssistant

_LOGGER = logging.getLogger(__name__)


async def async_list_views(hass: HomeAssistant) -> list[tuple[str, str]]:
    """Return [(path, label)], e.g. ("/lovelace/0", "Overview › Home"). Default dashboard first.

    Dashboards that require admin are skipped: the renderer logs in as a read-only user.
    """
    data = hass.data.get(LOVELACE_DATA)
    if data is None:
        return [("/lovelace/0", "Overview")]
    out: list[tuple[str, str]] = []
    keys = sorted(data.dashboards, key=lambda k: (k is not None, k or ""))
    for url_path in keys:
        dash = data.dashboards[url_path]
        meta = getattr(dash, "config", None) or {}
        if meta.get("require_admin"):
            continue
        url = url_path or "lovelace"
        title = meta.get("title") or ("Overview" if url_path is None else url)
        try:
            cfg = await dash.async_load(False)
        except Exception:  # noqa: BLE001 - auto-generated dashboards raise ConfigNotFound
            cfg = {}
        views = cfg.get("views") if isinstance(cfg, dict) else None
        if not views:
            out.append((f"/{url}/0", f"{title} (auto-generated)" if not cfg else title))
            continue
        for i, view in enumerate(views):
            vpath = view.get("path") or str(i)
            vtitle = view.get("title") or (view.get("path") or f"View {i + 1}")
            out.append((f"/{url}/{vpath}", f"{title} › {vtitle}"))
    return out
