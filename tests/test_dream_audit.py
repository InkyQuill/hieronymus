from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dream_audit import DreamAuditStore


def _dream_run_id(config: HieronymusConfig, *, cycle_id: int = 1) -> int:
    with connect(config.database_path) as conn:
        cursor = conn.execute(
            """
            insert into dream_runs(cycle_id, status, provider, created_at)
            values (?, 'running', 'test', '2026-06-09T00:00:00+00:00')
            """,
            (cycle_id,),
        )
        conn.commit()
    return int(cursor.lastrowid)


def _phase_run_id(config: HieronymusConfig, dream_run_id: int) -> int:
    with connect(config.database_path) as conn:
        cursor = conn.execute(
            """
            insert into dream_phase_runs(
              dream_run_id,
              phase,
              provider_profile,
              provider_type,
              model,
              status,
              created_at
            )
            values (?, 'extract', 'default', 'anthropic', 'claude-test', 'running', ?)
            """,
            (dream_run_id, "2026-06-09T00:00:00+00:00"),
        )
        conn.commit()
    return int(cursor.lastrowid)


def test_append_list_roundtrip(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)
    phase_run_id = _phase_run_id(config, dream_run_id)

    entry_id = store.append(
        dream_run_id=dream_run_id,
        phase_run_id=phase_run_id,
        event_type="provider_request",
        severity="info",
        summary="queued crystallization request",
        payload={"model": "claude-test", "temperature": 0.2},
    )

    entries = store.list_for_run(dream_run_id)
    assert len(entries) == 1
    assert entries[0].id == entry_id
    assert entries[0].dream_run_id == dream_run_id
    assert entries[0].phase_run_id == phase_run_id
    assert entries[0].event_type == "provider_request"
    assert entries[0].severity == "info"
    assert entries[0].summary == "queued crystallization request"
    assert entries[0].payload == {"model": "claude-test", "temperature": 0.2}
    assert entries[0].created_at


def test_append_redacts_nested_secret_keys(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="provider_request",
        severity="info",
        summary="sent request",
        payload={
            "model": "claude-test",
            "headers": {
                "Authorization": "Bearer secret",
                "anthropic-version": "2023-06-01",
                "x-api-key": "secret-key",
            },
            "messages": [
                {"role": "user", "token": "nested-secret", "content": "safe"},
                {"apiKey": "camel-secret", "bearer": "bearer-secret"},
            ],
        },
    )

    payload = store.list_for_run(dream_run_id)[0].payload
    assert payload == {
        "headers": {
            "Authorization": "[REDACTED]",
            "anthropic-version": "[REDACTED]",
            "x-api-key": "[REDACTED]",
        },
        "messages": [
            {"content": "safe", "role": "user", "token": "[REDACTED]"},
            {"apiKey": "[REDACTED]", "bearer": "[REDACTED]"},
        ],
        "model": "claude-test",
    }


def test_append_redacts_gemini_api_key_header(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="provider_request",
        severity="info",
        summary="sent gemini request",
        payload={
            "model": "gemini-2.5-pro",
            "headers": {"x-goog-api-key": "gemini-secret"},
        },
    )

    assert store.list_for_run(dream_run_id)[0].payload == {
        "headers": {"x-goog-api-key": "[REDACTED]"},
        "model": "gemini-2.5-pro",
    }


def test_append_redacts_secret_keys_inside_tuples(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="provider_request",
        severity="info",
        summary="sent tuple payload",
        payload={
            "messages": (
                {"role": "user", "content": "safe"},
                {"Authorization": "Bearer tuple-secret", "model": "claude-test"},
            )
        },
    )

    assert store.list_for_run(dream_run_id)[0].payload == {
        "messages": [
            {"content": "safe", "role": "user"},
            {"Authorization": "[REDACTED]", "model": "claude-test"},
        ]
    }


def test_append_redacts_nested_token_like_keys(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="provider_response",
        severity="info",
        summary="received token payload",
        payload={
            "provider": {
                "model": "oauth-model",
                "access_token": "access-secret",
                "refresh_token": "refresh-secret",
            },
            "headers": [{"bearer_token": "bearer-secret", "status": "safe"}],
        },
    )

    assert store.list_for_run(dream_run_id)[0].payload == {
        "headers": [{"bearer_token": "[REDACTED]", "status": "safe"}],
        "provider": {
            "access_token": "[REDACTED]",
            "model": "oauth-model",
            "refresh_token": "[REDACTED]",
        },
    }


def test_list_for_run_orders_entries_by_id(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    first_id = store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="first",
        severity="info",
        summary="first entry",
        payload={},
    )
    second_id = store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="second",
        severity="warning",
        summary="second entry",
        payload={},
    )

    assert [entry.id for entry in store.list_for_run(dream_run_id)] == [first_id, second_id]


def test_phase_run_id_can_be_none(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)
    dream_run_id = _dream_run_id(config)

    store.append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="run_started",
        severity="info",
        summary="started run",
        payload={"model": "deterministic"},
    )

    assert store.list_for_run(dream_run_id)[0].phase_run_id is None
