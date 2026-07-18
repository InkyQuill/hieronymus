import asyncio

import pytest

import hieronymus.daemon_events as daemon_events

AdminEventHub = daemon_events.AdminEventHub


def test_event_hub_delivers_json_safe_events_to_subscribers() -> None:
    async def exercise() -> None:
        hub = AdminEventHub()
        subscription = hub.subscribe()

        hub.publish("dream_started", {"run_id": 4, "phase": "knowledge_crystals"})
        event = await asyncio.wait_for(subscription.receive(), timeout=1)
        subscription.close()
        hub.publish("dream_completed", {"run_id": 4})

        assert event["type"] == "dream_started"
        assert event["payload"] == {"run_id": 4, "phase": "knowledge_crystals"}
        assert isinstance(event["timestamp"], str)
        assert hub.subscriber_count == 0

    asyncio.run(exercise())


def test_event_hub_overflow_disconnects_only_the_slow_subscriber() -> None:
    async def exercise() -> None:
        hub = AdminEventHub(default_capacity=1)
        slow = hub.subscribe()
        healthy = hub.subscribe()

        hub.publish("dream_started", {"run_id": 4})
        assert (await asyncio.wait_for(healthy.receive(), timeout=1))["type"] == "dream_started"
        hub.publish("dream_phase_progress", {"run_id": 4})
        assert (await asyncio.wait_for(healthy.receive(), timeout=1))["type"] == (
            "dream_phase_progress"
        )

        with pytest.raises(daemon_events.EventSubscriptionOverflow):
            await asyncio.wait_for(slow.receive(), timeout=1)
        assert hub.subscriber_count == 1

        hub.publish("dream_completed", {"run_id": 4})
        assert (await asyncio.wait_for(healthy.receive(), timeout=1))["type"] == ("dream_completed")
        healthy.close()
        assert hub.subscriber_count == 0

    asyncio.run(exercise())


def test_event_hub_closed_subscriber_does_not_affect_other_subscribers() -> None:
    async def exercise() -> None:
        hub = AdminEventHub()
        dead = hub.subscribe()
        healthy = hub.subscribe()
        dead.close()

        hub.publish("dream_completed", {"run_id": 4})

        assert (await asyncio.wait_for(healthy.receive(), timeout=1))["type"] == ("dream_completed")
        healthy.close()

    asyncio.run(exercise())
