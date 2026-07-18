import asyncio
import threading

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


def test_concurrent_publishers_preserve_one_global_order_for_all_subscribers(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    async def exercise() -> None:
        hub = AdminEventHub()
        first = hub.subscribe()
        second = hub.subscribe()
        first_offer_started = threading.Event()
        second_publish_offered = threading.Event()
        release_first_offer = threading.Event()
        original_offer = daemon_events.AdminEventSubscription.offer
        blocked = False

        def controlled_offer(self, event: dict[str, object]) -> None:
            nonlocal blocked
            original_offer(self, event)
            if event["payload"] == {"sequence": 2}:
                second_publish_offered.set()
            if event["payload"] == {"sequence": 1} and not blocked:
                blocked = True
                first_offer_started.set()
                assert release_first_offer.wait(timeout=1)

        monkeypatch.setattr(daemon_events.AdminEventSubscription, "offer", controlled_offer)
        publisher_one = threading.Thread(
            target=hub.publish,
            args=("progress", {"sequence": 1}),
        )
        publisher_two = threading.Thread(
            target=hub.publish,
            args=("progress", {"sequence": 2}),
        )
        publisher_one.start()
        assert first_offer_started.wait(timeout=1)
        publisher_two.start()
        second_publish_offered.wait(timeout=0.2)
        release_first_offer.set()
        publisher_one.join(timeout=1)
        publisher_two.join(timeout=1)
        assert not publisher_one.is_alive()
        assert not publisher_two.is_alive()

        observed = []
        for subscription in (first, second):
            observed.append(
                [
                    (await asyncio.wait_for(subscription.receive(), timeout=1))["payload"][
                        "sequence"
                    ]
                    for _ in range(2)
                ]
            )
            subscription.close()

        assert observed == [[1, 2], [1, 2]]

    asyncio.run(exercise())
