import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { bootstrapSession, takeLaunchGrant } from "./bootstrap";

test("consumes grant from history before any network request", () => {
  const history = { replaceState: vi.fn() };
  expect(
    takeLaunchGrant(
      { hash: "#launch_grant=abc", pathname: "/admin", search: "" },
      history,
    ),
  ).toBe("abc");
  expect(history.replaceState).toHaveBeenCalledWith(null, "", "/admin");
});

test("takeLaunchGrant preserves an existing query string when scrubbing", () => {
  const history = { replaceState: vi.fn() };
  expect(
    takeLaunchGrant(
      {
        hash: "#launch_grant=xyz&other=1",
        pathname: "/config",
        search: "?tab=dream",
      },
      history,
    ),
  ).toBe("xyz");
  expect(history.replaceState).toHaveBeenCalledWith(
    null,
    "",
    "/config?tab=dream",
  );
});

test("takeLaunchGrant leaves history untouched when no grant is present", () => {
  const history = { replaceState: vi.fn() };
  expect(
    takeLaunchGrant({ hash: "", pathname: "/admin", search: "" }, history),
  ).toBeNull();
  expect(history.replaceState).not.toHaveBeenCalled();
});

const originalHash = window.location.hash;

beforeEach(() => {
  const app = document.createElement("div");
  app.id = "app";
  document.body.replaceChildren(app);
});

afterEach(() => {
  vi.restoreAllMocks();
  try {
    window.history.replaceState(null, "", `/${originalHash}`);
  } catch {
    // a test may have left replaceState stubbed to throw; ignore
  }
  vi.unstubAllGlobals();
});

test("bootstrapSession exchanges the grant exactly once, after scrubbing the fragment", async () => {
  window.history.replaceState(null, "", "/admin#launch_grant=secret-grant");
  const calls: { url: string; body: string; hashAtCall: string }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit) => {
      calls.push({
        url,
        body: String(init.body),
        hashAtCall: window.location.hash,
      });
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    }),
  );

  await bootstrapSession();

  expect(calls).toHaveLength(1);
  expect(calls[0].url).toBe("/auth/launch-grant/exchange");
  expect(JSON.parse(calls[0].body)).toEqual({ launch_grant: "secret-grant" });
  // The fragment was removed synchronously before the await.
  expect(calls[0].hashAtCall).toBe("");
  expect(window.location.hash).toBe("");
  // The grant is never persisted.
  expect(window.localStorage.getItem("launch_grant")).toBeNull();
  expect(document.getElementById("app")!.textContent).toBe("");
});

test("bootstrapSession does not call the network when there is no grant", async () => {
  window.history.replaceState(null, "", "/admin");
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);

  await bootstrapSession();

  expect(fetchMock).not.toHaveBeenCalled();
});

test("bootstrapSession renders a relaunch instruction and rejects on a spent grant", async () => {
  window.history.replaceState(null, "", "/admin#launch_grant=stale");
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () =>
        new Response(JSON.stringify({ error: "launch_grant_already_used" }), {
          status: 401,
        }),
    ),
  );

  await expect(bootstrapSession()).rejects.toThrow();

  const app = document.getElementById("app")!;
  expect(app.textContent).toMatch(/hiero admin/);
  expect(app.textContent).not.toBe("");
});

test("bootstrapSession treats an empty launch_grant as no grant", async () => {
  window.history.replaceState(null, "", "/admin#launch_grant=");
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);

  await bootstrapSession();

  expect(fetchMock).not.toHaveBeenCalled();
});

test("bootstrapSession still exchanges the grant when history.replaceState throws", async () => {
  window.location.hash = "#launch_grant=sandboxed-grant";
  vi.spyOn(window.history, "replaceState").mockImplementation(() => {
    throw new DOMException("sandboxed", "SecurityError");
  });
  const bodies: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      bodies.push(String(init.body));
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    }),
  );

  await bootstrapSession();

  expect(bodies).toHaveLength(1);
  expect(JSON.parse(bodies[0])).toEqual({ launch_grant: "sandboxed-grant" });
});

test("bootstrapSession renders a relaunch instruction when the network fails", async () => {
  window.history.replaceState(null, "", "/config#launch_grant=stale");
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      throw new TypeError("network down");
    }),
  );

  await expect(bootstrapSession()).rejects.toThrow();
  expect(document.getElementById("app")!.textContent).toMatch(/hiero admin/);
});
