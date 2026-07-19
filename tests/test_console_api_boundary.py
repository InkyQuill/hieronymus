import importlib.util

import hieronymus.console_api as console_api
from hieronymus.console_api import AdminBridge, ConfigBridge, error_payload


def test_console_api_exports_typed_services_and_error_payloads() -> None:
    assert console_api.__all__ == ["AdminBridge", "ConfigBridge", "error_payload"]
    assert callable(AdminBridge.bootstrap)
    assert callable(ConfigBridge.bootstrap)
    assert error_payload(ValueError("invalid setting")) == {
        "code": "validation_error",
        "message": "invalid setting",
    }
    assert error_payload(
        ValueError("provider rejected raw-secret-value"),
        redact=lambda message: message.replace("raw-secret-value", "[redacted]"),
    ) == {
        "code": "validation_error",
        "message": "provider rejected [redacted]",
    }


def test_console_api_does_not_export_obsolete_stdio_entrypoint() -> None:
    obsolete_entrypoint = "run_" + "stdio"
    assert not hasattr(console_api, obsolete_entrypoint)


def test_obsolete_console_namespace_is_not_importable() -> None:
    obsolete_namespace = "hieronymus.tui" + "_bridge"
    assert importlib.util.find_spec(obsolete_namespace) is None
