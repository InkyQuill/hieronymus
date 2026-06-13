import React from "react";
import { act } from "react";
import type { ParsedKey } from "@opentui/core";
import type {
  TestRendererOptions,
  TestRendererSetup,
} from "@opentui/core/testing";
import { testRender } from "@opentui/react/test-utils";

type KeyOptions = {
  ctrl?: boolean;
  shift?: boolean;
};

type Harness = {
  render: (node: React.ReactNode) => Promise<void>;
  flush: () => Promise<void>;
  waitFor: (
    predicate: () => boolean | Promise<boolean>,
    maxPasses?: number,
  ) => Promise<void>;
  waitForFrame: (
    predicate: (frame: string) => boolean | Promise<boolean>,
    maxPasses?: number,
  ) => Promise<string>;
  captureCharFrame: () => string;
  mockInput: {
    press: (name: string, options?: KeyOptions) => Promise<void>;
    type: (value: string) => Promise<void>;
  };
  cleanup: () => Promise<void>;
};

const activeHarnesses = new Set<Harness>();

export async function cleanupOpenTuiHarnesses(): Promise<void> {
  const harnesses = Array.from(activeHarnesses);
  const failures: unknown[] = [];

  for (const harness of harnesses) {
    try {
      await harness.cleanup();
    } catch (error) {
      failures.push(error);
    } finally {
      activeHarnesses.delete(harness);
    }
  }

  if (failures.length === 1) {
    throw failures[0];
  }
  if (failures.length > 1) {
    throw new AggregateError(
      failures,
      "Failed to clean up OpenTUI test harnesses",
    );
  }
}

export function createOpenTuiHarness(
  options: TestRendererOptions,
): Harness {
  let setup: TestRendererSetup | null = null;

  const ensureSetup = () => {
    if (!setup) {
      throw new Error("OpenTUI test harness has not rendered yet");
    }
    return setup;
  };

  const render = async (node: React.ReactNode) => {
    if (setup) {
      await cleanup();
    }
    await act(async () => {
      setup = await testRender(node, options);
      activeHarnesses.add(harness);
      await setup.flush();
    });
  };

  const flush = async () => {
    const current = ensureSetup();
    await act(async () => {
      await current.flush();
    });
  };

  const waitFor = async (
    predicate: () => boolean | Promise<boolean>,
    maxPasses = 25,
  ) => {
    const current = ensureSetup();
    await act(async () => {
      await current.waitFor(predicate, { maxPasses });
    });
  };

  const waitForFrame = async (
    predicate: (frame: string) => boolean | Promise<boolean>,
    maxPasses = 25,
  ) => {
    const current = ensureSetup();
    let frame = "";
    await act(async () => {
      frame = await current.waitForFrame(predicate, { maxPasses });
    });
    return frame;
  };

  const press = async (name: string, options: KeyOptions = {}) => {
    const current = ensureSetup();
    await act(async () => {
      if (name === "enter") {
        current.mockInput.pressEnter(options);
      } else if (name === "tab") {
        current.mockInput.pressTab(options);
      } else if (name === "backspace") {
        current.mockInput.pressBackspace();
      } else if (name === "escape") {
        const escapeKey: ParsedKey = {
          name: "escape",
          ctrl: options.ctrl ?? false,
          meta: false,
          shift: options.shift ?? false,
          option: false,
          sequence: "\x1B",
          number: false,
          raw: "\x1B",
          eventType: "press",
          source: "raw",
        };
        current.renderer.keyInput.processParsedKey(escapeKey);
      } else if (
        name === "up" ||
        name === "down" ||
        name === "left" ||
        name === "right"
      ) {
        current.mockInput.pressArrow(name, options);
      } else {
        current.mockInput.pressKey(name, options);
      }
    });
    await flush();
  };

  const type = async (value: string) => {
    const current = ensureSetup();
    await act(async () => {
      for (const key of value) {
        current.mockInput.pressKey(key);
      }
    });
    await flush();
  };

  const captureCharFrame = () => setup?.captureCharFrame() ?? "";

  const cleanup = async () => {
    if (!setup) {
      return;
    }
    const current = setup;
    setup = null;
    activeHarnesses.delete(harness);
    await act(async () => {
      current.renderer.destroy();
      await Promise.resolve();
    });
  };

  const harness: Harness = {
    render,
    flush,
    waitFor,
    waitForFrame,
    captureCharFrame,
    mockInput: { press, type },
    cleanup,
  };

  return harness;
}
