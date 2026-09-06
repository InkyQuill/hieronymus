/**
 * One-time browser session bootstrap (ADR 0012, 2026-09-03 amendment).
 *
 * `hiero admin` / `hiero config` mint a 60-second, single-use launch grant and
 * hand it to the browser in the URL *fragment* of the console tab they open.
 * A fragment is never sent to the server and never written to server logs; the
 * amendment only prohibits query-string secrets, so this is the sanctioned
 * transport. {@link takeLaunchGrant} removes the fragment synchronously — with
 * `history.replaceState`, before any `await` — then {@link bootstrapSession}
 * exchanges it once for the `HttpOnly` session cookie. The grant is never
 * stored; a reload with no fragment simply relies on the cookie set by the
 * first exchange.
 */

const EXCHANGE_PATH = "/auth/launch-grant/exchange";

/**
 * Shown in `#app` when there is no usable session: the browser cannot mint its
 * own grant, so the only recovery is to relaunch from the CLI.
 */
const RELAUNCH_INSTRUCTION =
  "This console could not start an authenticated session. " +
  "Run `hiero admin` (or `hiero config`) again in your terminal to open a fresh console tab.";

/**
 * Read a `launch_grant` from `location.hash` and, if present, scrub it from the
 * address bar immediately (synchronous `history.replaceState`) so it never
 * survives into a later navigation, reload, or `Referer`. Returns the grant, or
 * `null` when the fragment carries none.
 *
 * The scrub is best-effort: some sandboxed embeds throw on `history.replaceState`.
 * A throw there must not blank the page or lose the grant, so it is swallowed —
 * {@link bootstrapSession} still exchanges the grant and still renders the
 * relaunch instruction on any failure.
 */
export function takeLaunchGrant(
  location: Pick<Location, "hash" | "pathname" | "search">,
  history: Pick<History, "replaceState">,
): string | null {
  const grant = new URLSearchParams(location.hash.slice(1)).get("launch_grant");
  if (grant !== null) {
    try {
      history.replaceState(null, "", location.pathname + location.search);
    } catch {
      // Sandboxed context: the fragment stays in the URL, but the grant is
      // single-use and about to be spent by the exchange below.
    }
  }
  return grant;
}

/**
 * Consume the launch grant (if any) before the app mounts. On a successful
 * exchange the daemon sets the session cookie and this resolves. On a missing
 * or spent grant it renders {@link RELAUNCH_INSTRUCTION} into `#app` and
 * rejects, so the caller mounts nothing — never an empty dashboard.
 */
export async function bootstrapSession(): Promise<void> {
  const grant = takeLaunchGrant(window.location, window.history);
  if (!grant) {
    // No grant in this navigation (absent, or an empty `#launch_grant=`): a
    // prior exchange may already have set the session cookie. Let the app load
    // and surface any 401 itself.
    return;
  }

  let response: Response;
  try {
    response = await fetch(EXCHANGE_PATH, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ launch_grant: grant }),
    });
  } catch (cause) {
    renderRelaunchInstruction();
    throw new Error("launch grant exchange could not reach the daemon", {
      cause,
    });
  }

  if (!response.ok) {
    renderRelaunchInstruction();
    throw new Error(`launch grant exchange failed (${response.status})`);
  }
}

function renderRelaunchInstruction(): void {
  const target = document.getElementById("app");
  if (target === null) {
    return;
  }
  const message = document.createElement("p");
  message.textContent = RELAUNCH_INSTRUCTION;
  target.replaceChildren(message);
}
