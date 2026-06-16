from __future__ import annotations

from collections.abc import Callable

from hieronymus.provider_config import ProviderCatalog
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.tui_bridge.protocol import RpcError


def error_code(error: Exception) -> str:
    if isinstance(error, RpcError):
        return error.code
    if isinstance(error, ValueError | KeyError):
        return "validation_error"
    return "internal_error"


def display_message(
    error: Exception,
    *,
    provider_catalog: ProviderCatalog | None = None,
    redact: Callable[[str], str] | None = None,
) -> str:
    if isinstance(error, RpcError):
        message = error.message
    elif isinstance(error, KeyError) and error.args:
        message = str(error.args[0])
    elif isinstance(error, ValueError):
        message = str(error)
    else:
        message = "Unexpected backend error"
    if redact is not None:
        return redact(message)
    if provider_catalog is not None:
        return redact_configured_secret_values(message, provider_catalog)
    return message


def error_payload(
    error: Exception,
    *,
    provider_catalog: ProviderCatalog | None = None,
    redact: Callable[[str], str] | None = None,
) -> dict[str, str]:
    return {
        "code": error_code(error),
        "message": display_message(error, provider_catalog=provider_catalog, redact=redact),
    }
