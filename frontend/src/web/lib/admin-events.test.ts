import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { acceptEvent, connectAdminEvents } from "./admin-events.svelte";

// --- acceptEvent: reduce a wire frame to "does the view need to refetch?" ---

test("does not refresh twice for replay/live duplicate delivery", () => {
  const event = {
    version: 1,
    event_id: 9,
    event_type: "dream_phase_completed",
    payload: {},
  };
  expect(acceptEvent(8, event)).toEqual({ last: 9, refresh: true });
  expect(acceptEvent(9, event)).toEqual({ last: 9, refresh: false });
});

test.each([
  ["null", null],
  ["a number", 42],
  ["a string", "dream_completed"],
  ["an array", []],
  ["a wrong protocol version", { version: 2, event_id: 1, event_type: "x" }],
  ["a string event_id", { version: 1, event_id: "1", event_type: "x" }],
  ["a fractional event_id", { version: 1, event_id: 1.5, event_type: "x" }],
  ["a negative event_id", { version: 1, event_id: -1, event_type: "x" }],
  [
    "an unsafe-integer event_id",
    { version: 1, event_id: Number.MAX_SAFE_INTEGER + 1, event_type: "x" },
  ],
  ["a missing event_id", { version: 1, event_type: "x" }],
  ["a missing event_type", { version: 1, event_id: 1 }],
])(
  "a malformed frame (%s) never moves the cursor or refreshes",
  (_label, value) => {
    expect(acceptEvent(7, value)).toEqual({ last: 7, refresh: false });
  },
);

test("snapshot_refresh always refreshes and jumps the cursor to its id, even when the cursor looks current", () => {
  expect(
    acceptEvent(1000, {
      version: 1,
      event_id: 256,
      event_type: "snapshot_refresh",
      payload: {},
    }),
  ).toEqual({ last: 256, refresh: true });
});

test("after a retained-history gap the client resumes cleanly from the snapshot_refresh id", () => {
  let cursor = 5;
  ({ last: cursor } = acceptEvent(cursor, {
    version: 1,
    event_id: 260,
    event_type: "snapshot_refresh",
    payload: {},
  }));
  expect(cursor).toBe(260);
  expect(
    acceptEvent(cursor, {
      version: 1,
      event_id: 261,
      event_type: "dream_completed",
      payload: {},
    }),
  ).toEqual({ last: 261, refresh: true });
});

test("a stale out-of-order event below the cursor is discarded without a refresh", () => {
  expect(acceptEvent(0, ev(1))).toEqual({ last: 1, refresh: true });
  expect(acceptEvent(3, ev(2))).toEqual({ last: 3, refresh: false });
  expect(acceptEvent(3, ev(3))).toEqual({ last: 3, refresh: false });
});

// --- connectAdminEvents: cursor, dedup, resume-on-reconnect, timer cleanup ---

function ev(id: number, type = "dream_phase_progress") {
  return { version: 1, event_id: id, event_type: type, payload: {} };
}

class FakeSocket {
  static instances: FakeSocket[] = [];
  url: string;
  sent: string[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeSocket.instances.push(this);
  }

  send(data: string) {
    if (this.closed) throw new Error("socket is closed");
    this.sent.push(data);
  }

  close() {
    this.closed = true;
  }

  open() {
    this.onopen?.();
  }

  deliver(frame: unknown) {
    this.onmessage?.({
      data: typeof frame === "string" ? frame : JSON.stringify(frame),
    });
  }

  drop() {
    this.onclose?.();
  }
}

beforeEach(() => {
  FakeSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeSocket);
});

afterEach(() => {
  vi.useRealTimers();
});

test("connectAdminEvents does not refresh on socket open and sends no resume on the first connect", () => {
  const onEvent = vi.fn();
  const stop = connectAdminEvents(onEvent);
  const socket = FakeSocket.instances[0];
  socket.open();
  expect(onEvent).not.toHaveBeenCalled();
  expect(socket.sent).toEqual([]);
  stop();
});

test("a dropped socket reconnects and resumes from the last delivered event id, then ignores the replay duplicate", () => {
  vi.useFakeTimers();
  const onEvent = vi.fn();
  const stop = connectAdminEvents(onEvent);

  const first = FakeSocket.instances[0];
  first.open();
  first.deliver(ev(4));
  expect(onEvent).toHaveBeenCalledTimes(1);

  first.drop();
  vi.advanceTimersByTime(250);

  const second = FakeSocket.instances[1];
  expect(second).toBeDefined();
  second.open();
  expect(second.sent).toEqual([JSON.stringify({ resume_from_event_id: 4 })]);

  // The hub replays event 4 as part of the catch-up: it must not refresh again.
  second.deliver(ev(4));
  expect(onEvent).toHaveBeenCalledTimes(1);
  // The next genuinely new event does refresh.
  second.deliver(ev(5, "dream_completed"));
  expect(onEvent).toHaveBeenCalledTimes(2);
  stop();
});

test("a fresh client (new daemon session / page reload) starts at cursor 0 and does not send a stale resume", () => {
  const stopOld = connectAdminEvents(vi.fn());
  const old = FakeSocket.instances[0];
  old.open();
  old.deliver(ev(99, "dream_completed"));
  stopOld();

  const onEvent = vi.fn();
  const stop = connectAdminEvents(onEvent);
  const fresh = FakeSocket.instances[1];
  fresh.open();
  expect(fresh.sent).toEqual([]);
  // The restarted daemon numbers its first event 1 again; the fresh client
  // must treat it as new, not as a stale duplicate of the old id 99.
  fresh.deliver(ev(1, "dream_started"));
  expect(onEvent).toHaveBeenCalledTimes(1);
  stop();
});

test("cleanup cancels a pending reconnect timer so no socket is reopened", () => {
  vi.useFakeTimers();
  const stop = connectAdminEvents(vi.fn());
  const socket = FakeSocket.instances[0];
  socket.open();
  socket.drop(); // schedules a reconnect
  stop(); // must cancel it

  vi.advanceTimersByTime(60_000);
  expect(FakeSocket.instances).toHaveLength(1);
});

test("a malformed socket frame is ignored without throwing or refreshing", () => {
  const onEvent = vi.fn();
  const stop = connectAdminEvents(onEvent);
  const socket = FakeSocket.instances[0];
  socket.open();
  expect(() => socket.deliver("not json{")).not.toThrow();
  socket.deliver({ version: 2, event_id: 1, event_type: "x" });
  expect(onEvent).not.toHaveBeenCalled();
  stop();
});

test("reconnect backoff grows on repeated drops and resets after a successful open", () => {
  vi.useFakeTimers();
  const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");
  const stop = connectAdminEvents(vi.fn());
  const lastDelay = () => setTimeoutSpy.mock.calls.at(-1)?.[1];

  FakeSocket.instances[0].drop();
  expect(lastDelay()).toBe(250);
  vi.advanceTimersByTime(250);
  FakeSocket.instances[1].drop();
  expect(lastDelay()).toBe(500);
  vi.advanceTimersByTime(500);
  FakeSocket.instances[2].open(); // a successful open resets the backoff
  FakeSocket.instances[2].drop();
  expect(lastDelay()).toBe(250);
  stop();
});

test("a frame that arrives after the cleanup function ran does not dispatch a refresh", () => {
  const onEvent = vi.fn();
  const stop = connectAdminEvents(onEvent);
  const socket = FakeSocket.instances[0];
  socket.open();
  stop();
  socket.deliver(ev(1));
  expect(onEvent).not.toHaveBeenCalled();
});

test("a throwing onEvent callback does not wedge processing of the next frame", () => {
  const onEvent = vi.fn(() => {
    throw new Error("consumer blew up");
  });
  const stop = connectAdminEvents(onEvent);
  const socket = FakeSocket.instances[0];
  socket.open();
  expect(() => socket.deliver(ev(1))).toThrow("consumer blew up");
  // The cursor advanced before onEvent() threw: a replayed duplicate is still
  // deduped (no throw), and the next genuinely new event still dispatches.
  expect(() => socket.deliver(ev(1))).not.toThrow();
  expect(() => socket.deliver(ev(2, "dream_completed"))).toThrow(
    "consumer blew up",
  );
  expect(onEvent).toHaveBeenCalledTimes(2);
  stop();
});
