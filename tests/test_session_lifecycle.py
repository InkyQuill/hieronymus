from datetime import UTC, datetime, timedelta

from hieronymus.session_lifecycle import SessionLifecycle
from hieronymus.workspace import WorkspaceStore


def test_lifecycle_closes_stale_sessions_before_threshold_check(config, monkeypatch) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(__import__("test_workspace")._context(config))
    now = datetime.now(UTC)
    from hieronymus.db import connect

    with connect(config.database_path) as conn:
        conn.execute(
            "update task_sessions set last_activity_at = ? where id = ?",
            ((now - timedelta(minutes=31)).isoformat(), session.id),
        )
        conn.commit()

    checked = []
    lifecycle = SessionLifecycle(config, threshold_check=lambda: checked.append(True))
    assert lifecycle.run_due(now) == (session.id,)
    assert checked == [True]
