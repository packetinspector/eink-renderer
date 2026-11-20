"""The Home Assistant user + token the renderer logs in with.

A dedicated user "E-ink Renderer": read-only group, local network only, no password (cannot log in
interactively). Visible under Settings -> People -> Users; removed together with the config entry.
"""

from __future__ import annotations

from datetime import timedelta

from homeassistant.auth.const import GROUP_ID_READ_ONLY
from homeassistant.auth.models import TOKEN_TYPE_LONG_LIVED_ACCESS_TOKEN
from homeassistant.core import HomeAssistant

from .const import USER_NAME

TOKEN_LIFETIME = timedelta(days=3650)


async def async_create_credentials(hass: HomeAssistant, label: str) -> tuple[str, str]:
    """Create the renderer user and a long-lived token. Returns (user_id, token)."""
    user = await hass.auth.async_create_user(USER_NAME, group_ids=[GROUP_ID_READ_ONLY], local_only=True)
    refresh = await hass.auth.async_create_refresh_token(
        user,
        client_name=f"eink-renderer {label}",
        token_type=TOKEN_TYPE_LONG_LIVED_ACCESS_TOKEN,
        access_token_expiration=TOKEN_LIFETIME,
    )
    return user.id, hass.auth.async_create_access_token(refresh)


async def async_new_token(hass: HomeAssistant, user_id: str, label: str) -> str | None:
    """A fresh token for an existing renderer user (re-pairing)."""
    user = await hass.auth.async_get_user(user_id)
    if user is None:
        return None
    for rt in list(user.refresh_tokens.values()):
        if rt.token_type == TOKEN_TYPE_LONG_LIVED_ACCESS_TOKEN:
            hass.auth.async_remove_refresh_token(rt)
    refresh = await hass.auth.async_create_refresh_token(
        user,
        client_name=f"eink-renderer {label}",
        token_type=TOKEN_TYPE_LONG_LIVED_ACCESS_TOKEN,
        access_token_expiration=TOKEN_LIFETIME,
    )
    return hass.auth.async_create_access_token(refresh)


async def async_remove_credentials(hass: HomeAssistant, user_id: str | None) -> None:
    """Delete the renderer user (revokes its tokens)."""
    if not user_id:
        return
    user = await hass.auth.async_get_user(user_id)
    if user is not None:
        await hass.auth.async_remove_user(user)
