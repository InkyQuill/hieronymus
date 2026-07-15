import { describe, expect, it } from "bun:test";
import { theme } from "./theme.js";

describe("theme", () => {
  it("exposes the semantic color slots used across the admin and config screens", () => {
    expect(theme.accentPrimary).toBe("cyan");
    expect(theme.accentMuted).toBe("gray");
    expect(theme.statusError).toBe("red");
    expect(theme.statusSuccess).toBe("green");
    expect(theme.statusWarning).toBe("yellow");
  });

  it("freezes the theme object so slots cannot be mutated at runtime", () => {
    expect(Object.isFrozen(theme)).toBe(true);
  });
});
