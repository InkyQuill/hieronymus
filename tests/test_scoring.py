import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.scoring import FeedbackStore, apply_score_delta
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
    )


def _add_crystal(
    config: HieronymusConfig,
    *,
    strength: float = 0.5,
    confidence: float = 0.5,
) -> int:
    return CrystalStore(config).add_crystal(
        _context(config),
        crystal_type="lesson",
        text="Use consistent Russian phrasing for crafting menu labels.",
        strength=strength,
        confidence=confidence,
    )


def test_feedback_session_context_match_is_accepted(config: HieronymusConfig) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Use consistent Russian phrasing for crafting menu labels.",
    )

    event_id = FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="confirmed_by_user",
        source_role="user",
        session_id=session.id,
    )

    with connect(config.database_path) as conn:
        row = conn.execute(
            "select crystal_id, session_id from memory_events where id = ?",
            (event_id,),
        ).fetchone()
    assert row["crystal_id"] == crystal_id
    assert row["session_id"] == session.id


def test_feedback_rejects_cross_series_session_without_event(
    config: HieronymusConfig,
) -> None:
    alpha_context = _context(config)
    Registry(config).create_series(
        slug="beta-series",
        title="Beta Series",
        source_language="ja",
        target_language="ru",
    )
    beta_context = TranslationContext(
        series_slug="beta-series",
        source_language="ja",
        target_language="ru",
        task_type="translate",
    )
    beta_session = WorkspaceStore(config).start_session(beta_context)
    crystal_id = CrystalStore(config).add_crystal(
        alpha_context,
        crystal_type="lesson",
        text="Use consistent Russian phrasing for crafting menu labels.",
    )

    with pytest.raises(ValueError, match="series_slug"):
        FeedbackStore(config).record(
            crystal_id=crystal_id,
            event_type="confirmed_by_user",
            source_role="user",
            session_id=beta_session.id,
        )

    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]
    assert event_count == 0


def test_feedback_rejects_unknown_session(config: HieronymusConfig) -> None:
    crystal_id = _add_crystal(config)

    with pytest.raises(KeyError, match="session"):
        FeedbackStore(config).record(
            crystal_id=crystal_id,
            event_type="confirmed_by_user",
            source_role="user",
            session_id=999,
        )


def test_user_confirmation_immediately_reinforces_crystal(config: HieronymusConfig) -> None:
    crystal_id = _add_crystal(config, strength=0.4, confidence=0.5)
    feedback = FeedbackStore(config)

    event_id = feedback.record(
        crystal_id=crystal_id,
        event_type="confirmed_by_user",
        source_role="user",
        evidence="Accepted during line review.",
    )

    crystal = CrystalStore(config).get(crystal_id)
    assert event_id > 0
    assert crystal.strength == pytest.approx(0.55)
    assert crystal.confidence == pytest.approx(0.7)
    assert crystal.status == "active"


def test_user_confirmation_accepts_positional_required_args(
    config: HieronymusConfig,
) -> None:
    crystal_id = _add_crystal(config, strength=0.4, confidence=0.5)

    event_id = FeedbackStore(config).record(crystal_id, "confirmed_by_user", "user")

    crystal = CrystalStore(config).get(crystal_id)
    assert event_id > 0
    assert crystal.strength == pytest.approx(0.55)
    assert crystal.confidence == pytest.approx(0.7)


def test_passive_event_is_recorded_but_not_applied_immediately(config: HieronymusConfig) -> None:
    crystal_id = _add_crystal(config, strength=0.4, confidence=0.5)

    FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="cited",
        source_role="mentor",
        evidence="Shown in recall context.",
    )

    crystal = CrystalStore(config).get(crystal_id)
    assert crystal.strength == pytest.approx(0.4)
    assert crystal.confidence == pytest.approx(0.5)


def test_user_deletion_archives_when_strength_falls_below_threshold(
    config: HieronymusConfig,
) -> None:
    crystal_id = _add_crystal(config, strength=0.4, confidence=0.5)

    FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="deleted_by_user",
        source_role="user",
        evidence="User removed stale lesson.",
    )

    crystal = CrystalStore(config).get(crystal_id)
    assert crystal.strength == 0.0
    assert crystal.confidence == pytest.approx(0.15)
    assert crystal.status == "archived"


def test_scores_are_clamped_to_zero_and_one(config: HieronymusConfig) -> None:
    high_id = _add_crystal(config, strength=0.95, confidence=0.95)
    low_id = _add_crystal(config, strength=0.1, confidence=0.1)
    feedback = FeedbackStore(config)

    feedback.record(
        crystal_id=high_id,
        event_type="confirmed_by_user",
        source_role="user",
    )
    feedback.record(
        crystal_id=low_id,
        event_type="deleted_by_user",
        source_role="user",
    )

    high = CrystalStore(config).get(high_id)
    low = CrystalStore(config).get(low_id)
    assert high.strength == 1.0
    assert high.confidence == 1.0
    assert low.strength == 0.0
    assert low.confidence == 0.0


def test_apply_score_delta_archives_zero_confidence_non_rule_but_not_active_rule() -> None:
    assert apply_score_delta(
        strength=0.02,
        confidence=0.01,
        status="active",
        crystal_type="lesson",
        strength_delta=-0.5,
        confidence_delta=-0.5,
    ) == (0.0, 0.0, "archived")
    assert apply_score_delta(
        strength=0.02,
        confidence=0.01,
        status="active",
        crystal_type="rule",
        strength_delta=-0.5,
        confidence_delta=-0.5,
    ) == (0.0, 0.0, "active")


def test_unknown_event_type_and_role_raise_value_error(config: HieronymusConfig) -> None:
    crystal_id = _add_crystal(config)
    feedback = FeedbackStore(config)

    with pytest.raises(ValueError, match="event_type"):
        feedback.record(
            crystal_id=crystal_id,
            event_type="praised",
            source_role="user",
        )
    with pytest.raises(ValueError, match="source_role"):
        feedback.record(
            crystal_id=crystal_id,
            event_type="confirmed_by_user",
            source_role="editor",
        )


def test_unknown_crystal_raises_key_error(config: HieronymusConfig) -> None:
    feedback = FeedbackStore(config)

    with pytest.raises(KeyError, match="crystal"):
        feedback.record(
            crystal_id=999,
            event_type="confirmed_by_user",
            source_role="user",
        )


def test_event_row_records_deltas_and_applied_flag(config: HieronymusConfig) -> None:
    crystal_id = _add_crystal(config)

    immediate_id = FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="contradicted_by_user",
        source_role="user",
        evidence="User corrected the suggested rendering.",
    )
    passive_id = FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="used_in_translation",
        source_role="system",
    )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select id, crystal_id, event_type, source_role, evidence,
                   strength_delta, confidence_delta, applied
            from memory_events
            order by id
            """
        ).fetchall()

    assert [row["id"] for row in rows] == [immediate_id, passive_id]
    assert rows[0]["crystal_id"] == crystal_id
    assert rows[0]["event_type"] == "contradicted_by_user"
    assert rows[0]["source_role"] == "user"
    assert rows[0]["evidence"] == "User corrected the suggested rendering."
    assert rows[0]["strength_delta"] == pytest.approx(-0.20)
    assert rows[0]["confidence_delta"] == pytest.approx(-0.25)
    assert rows[0]["applied"] == 1
    assert rows[1]["event_type"] == "used_in_translation"
    assert rows[1]["strength_delta"] == pytest.approx(0.05)
    assert rows[1]["confidence_delta"] == pytest.approx(0.02)
    assert rows[1]["applied"] == 0
