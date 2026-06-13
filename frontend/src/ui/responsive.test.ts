import { describe, expect, it } from "bun:test";
import { classifyTerminalLayout, panelWidth } from "./responsive.js";

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
    expect(classifyTerminalLayout(49, 20)).toEqual({
      kind: "too-small",
      width: 49,
      height: 20,
    });
  });

  it("keeps panel content inside the terminal width after borders", () => {
    expect(panelWidth({ kind: "compact", width: 80, height: 24 }, 2)).toBe(76);
    expect(panelWidth({ kind: "narrow", width: 60, height: 24 }, 2)).toBe(56);
  });
});
