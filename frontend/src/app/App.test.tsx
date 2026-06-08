import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { App } from "./App.js";
import type { JsonRpcClient } from "../rpc/client.js";

describe("App", () => {
  it("fails explicitly for admin mode until the admin Ink screen exists", () => {
    const app = render(<App mode="admin" client={fakeClient()} />);

    expect(app.lastFrame()).toContain("Admin Ink screen is not available yet");
  });
});

function fakeClient(): JsonRpcClient {
  return {
    request: () => Promise.reject(new Error("unexpected request")),
  } as unknown as JsonRpcClient;
}
