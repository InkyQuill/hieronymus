from __future__ import annotations

from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import HieronymusSettings, SettingsError
from hieronymus.tui_bridge.protocol import RpcError


def error_code(error: Exception) -> str:
    if isinstance(error, RpcError):
        return error.code
    if isinstance(error, SettingsError | ValueError | KeyError):
        return "validation_error"
    return "internal_error"


def display_message(
    error: Exception,
    *,
    settings: HieronymusSettings | None = None,
) -> str:
    if isinstance(error, RpcError):
        message = error.message
    elif isinstance(error, KeyError) and error.args:
        message = str(error.args[0])
    elif isinstance(error, SettingsError | ValueError):
        message = str(error)
    else:
        message = "Unexpected backend error"
    if settings is not None:
        return redact_configured_secret_values(message, settings)
    return message


def error_payload(
    error: Exception,
    *,
    settings: HieronymusSettings | None = None,
) -> dict[str, str]:
    return {
        "code": error_code(error),
        "message": display_message(error, settings=settings),
    }
