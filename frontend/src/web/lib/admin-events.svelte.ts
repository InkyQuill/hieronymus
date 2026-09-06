/**
 * Admin event-stream client (Task W4).
 *
 * The daemon's `/ws/admin` socket streams `{version, event_id, event_type,
 * payload}` envelopes for long-running admin work (dream runs, index rebuilds).
 * An event is only a hint that committed state changed — the durable record is
 * the audit log and the admin snapshot fetch — so the client never trusts a
 * payload as state. It reduces every frame to one question via {@link
 * acceptEvent}: does the reader's view need to refetch?
 *
 * The cursor is scoped to a single daemon session. A fresh page load / fresh
 * `bootstrapSession` constructs a fresh {@link connectAdminEvents} at cursor 0
 * and the cursor is never persisted (no `localStorage`): a restarted daemon
 * numbers its events from 1 again, and a stale cross-session cursor must not
 * suppress them. Within one page session a dropped socket reconnects and
 * resumes from the last-seen id with a `{"resume_from_event_id": N}` first
 * frame; the hub answers with the missed events or, when the client fell
 * outside the retained window, a single `snapshot_refresh`.
 */

const INITIAL_RETRY_MS = 250;
const MAX_RETRY_MS = 5_000;

/**
 * Fold one wire frame into `{ last, refresh }`. Malformed / wrong-version /
 * missing-field frames leave the cursor untouched and never refresh. A
 * `snapshot_refresh` always refreshes and re-anchors the cursor to its id
 * (the newest retained event). Any other event refreshes only when it is
 * strictly newer than the cursor, so replay/live duplicates are discarded.
 */
export function acceptEvent(
  last: number,
  value: unknown,
): { last: number; refresh: boolean } {
  if (typeof value !== "object" || value === null)
    return { last, refresh: false };
  const event = value as Record<string, unknown>;
  if (
    event.version !== 1 ||
    typeof event.event_id !== "number" ||
    !Number.isSafeInteger(event.event_id) ||
    event.event_id < 0 ||
    typeof event.event_type !== "string"
  )
    return { last, refresh: false };
  if (event.event_type === "snapshot_refresh") {
    return { last: event.event_id, refresh: true };
  }
  return {
    last: Math.max(last, event.event_id),
    refresh: event.event_id > last,
  };
}

export function connectAdminEvents(onEvent: () => void): () => void {
  let stopped = false;
  let socket: WebSocket | undefined;
  let retry = INITIAL_RETRY_MS;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  // Daemon-session-scoped, never persisted. Only a live reconnect resumes.
  let cursor = 0;
  let everOpened = false;

  function connect() {
    if (stopped) return;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(`${scheme}//${location.host}/ws/admin`);
    socket.onopen = () => {
      retry = INITIAL_RETRY_MS;
      // On a reconnect, ask the hub to replay everything committed since the
      // last id we saw. The first connect streams live from the next event —
      // the initial view load is the caller's responsibility, not ours.
      // When `everOpened && cursor === 0` (reconnected before seeing any
      // event) this sends `resume_from_event_id: 0`, i.e. a full retained
      // replay — the deliberate no-missed-events choice, deduped by cursor.
      if (everOpened) {
        try {
          socket?.send(JSON.stringify({ resume_from_event_id: cursor }));
        } catch {
          // A socket that cannot take the resume frame will close and retry.
        }
      }
      everOpened = true;
    };
    socket.onerror = () => {
      // Transient transport errors surface here; `onclose` drives the
      // reconnect, so this is a no-op that only keeps intent explicit and the
      // console quiet.
    };
    socket.onmessage = (message: MessageEvent) => {
      // A frame the browser had already buffered can fire after cleanup ran;
      // never dispatch a refresh into a torn-down consumer.
      if (stopped) return;
      let parsed: unknown;
      try {
        parsed = JSON.parse(
          typeof message.data === "string" ? message.data : "",
        );
      } catch {
        return;
      }
      const result = acceptEvent(cursor, parsed);
      cursor = result.last;
      if (result.refresh) onEvent();
    };
    socket.onclose = () => {
      if (stopped) return;
      reconnectTimer = setTimeout(connect, retry);
      retry = Math.min(retry * 2, MAX_RETRY_MS);
    };
  }

  connect();
  return () => {
    stopped = true;
    if (reconnectTimer !== undefined) clearTimeout(reconnectTimer);
    socket?.close();
  };
}
