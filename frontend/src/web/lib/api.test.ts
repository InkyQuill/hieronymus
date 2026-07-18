import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { REQUEST_TIMEOUT_MS, request } from "./api";

let requestSignal: AbortSignal | undefined;

function neverSettlingFetch(
  _input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  requestSignal = init?.signal ?? undefined;
  return new Promise((_resolve, reject) => {
    const rejectForAbort = () =>
      reject(
        requestSignal?.reason ?? new DOMException("Aborted", "AbortError"),
      );
    if (requestSignal?.aborted) rejectForAbort();
    else
      requestSignal?.addEventListener("abort", rejectForAbort, { once: true });
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  requestSignal = undefined;
  vi.stubGlobal("fetch", vi.fn(neverSettlingFetch));
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test("request aborts at the shared deadline with a stable timeout error", async () => {
  const result = expect(request("/api/slow")).rejects.toThrow(
    "Request timed out",
  );

  await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);

  await result;
  expect(requestSignal?.aborted).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});

test("request preserves caller cancellation during a request and cleans up", async () => {
  const controller = new AbortController();
  const abortReason = new DOMException(
    "Caller stopped the request",
    "AbortError",
  );
  const removeListener = vi.spyOn(controller.signal, "removeEventListener");
  const result = expect(
    request("/api/slow", {
      method: "POST",
      body: "payload",
      signal: controller.signal,
    }),
  ).rejects.toBe(abortReason);

  controller.abort(abortReason);

  await result;
  expect(fetch).toHaveBeenCalledWith(
    "/api/slow",
    expect.objectContaining({ method: "POST", body: "payload" }),
  );
  expect(removeListener).toHaveBeenCalledWith("abort", expect.any(Function));
  expect(vi.getTimerCount()).toBe(0);
});

test("request honors a caller signal that is already aborted", async () => {
  const controller = new AbortController();
  const abortReason = new DOMException("Already stopped", "AbortError");
  controller.abort(abortReason);

  await expect(
    request("/api/slow", { signal: controller.signal }),
  ).rejects.toBe(abortReason);
  expect(requestSignal?.aborted).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});
