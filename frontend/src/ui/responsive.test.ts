import { describe, expect, it } from "bun:test";
import {
  classifyTerminalLayout,
  panelHeight,
  panelWidth,
} from "./responsive.js";

describe("responsive layout helpers", () => {
  it("classifies wide, compact, narrow, and too-small terminal sizes", () => {
    expect(classifyTerminalLayout(160, 60)).toEqual({
      kind: "wide",
      width: 160,
      height: 60,
    });
    expect(classifyTerminalLayout(80, 24)).toEqual({
      kind: "compact",
      width: 80,
      height: 24,
    });
    expect(classifyTerminalLayout(60, 24)).toEqual({
      kind: "narrow",
      width: 60,
      height: 24,
    });
    expect(classifyTerminalLayout(60, 20)).toEqual({
      kind: "narrow",
      width: 60,
      height: 20,
    });
    expect(classifyTerminalLayout(79, 24)).toEqual({
      kind: "narrow",
      width: 79,
      height: 24,
    });
    expect(classifyTerminalLayout(80, 23)).toEqual({
      kind: "narrow",
      width: 80,
      height: 23,
    });
    expect(classifyTerminalLayout(132, 24)).toEqual({
      kind: "compact",
      width: 132,
      height: 24,
    });
    expect(classifyTerminalLayout(136, 24)).toEqual({
      kind: "wide",
      width: 136,
      height: 24,
    });
    expect(classifyTerminalLayout(136, 20)).toEqual({
      kind: "narrow",
      width: 136,
      height: 20,
    });
    expect(classifyTerminalLayout(59, 20)).toEqual({
      kind: "too-small",
      width: 59,
      height: 20,
    });
  });

  it("treats non-finite terminal dimensions as too small", () => {
    expect(classifyTerminalLayout(Number.NaN, 24).kind).toBe("too-small");
    expect(classifyTerminalLayout(80, Number.POSITIVE_INFINITY).kind).toBe(
      "too-small",
    );
  });

  it("keeps panel content inside the terminal width after borders", () => {
    expect(panelWidth({ kind: "compact", width: 80, height: 24 }, 2)).toBe(76);
    expect(panelWidth({ kind: "narrow", width: 60, height: 24 }, 2)).toBe(56);
  });

  it("keeps panel content inside the terminal height after reserved rows", () => {
    expect(panelHeight({ kind: "compact", width: 80, height: 24 }, 8)).toBe(16);
    expect(panelHeight({ kind: "narrow", width: 60, height: 20 }, 30)).toBe(4);
  });
});
