from hieronymus.daemon_events import AdminEventHub


def test_event_hub_delivers_json_safe_events_to_subscribers() -> None:
    received: list[dict[str, object]] = []
    hub = AdminEventHub()
    unsubscribe = hub.subscribe(received.append)

    hub.publish("dream_started", {"run_id": 4, "phase": "knowledge_crystals"})
    unsubscribe()
    hub.publish("dream_completed", {"run_id": 4})

    assert received[0]["type"] == "dream_started"
    assert received[0]["payload"] == {"run_id": 4, "phase": "knowledge_crystals"}
    assert isinstance(received[0]["timestamp"], str)
