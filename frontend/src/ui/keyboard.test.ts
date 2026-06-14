import { describe, expect, it } from "bun:test";
import {
  isConfirmKey,
  isDownKey,
  isEscapeKey,
  isLeftKey,
  isRightKey,
  isUpKey,
  printableSearchChar,
  type KeyboardInput,
} from "./keyboard.js";

function key(input: Partial<KeyboardInput> & { name: string }): KeyboardInput {
  return input;
}

describe("keyboard helpers", () => {
  it("detects arrow and hjkl movement keys", () => {
    expect(isUpKey(key({ name: "up" }))).toBe(true);
    expect(isUpKey(key({ name: "k" }))).toBe(true);
    expect(isDownKey(key({ name: "down" }))).toBe(true);
    expect(isDownKey(key({ name: "j" }))).toBe(true);
    expect(isLeftKey(key({ name: "left" }))).toBe(true);
    expect(isLeftKey(key({ name: "h" }))).toBe(true);
    expect(isRightKey(key({ name: "right" }))).toBe(true);
    expect(isRightKey(key({ name: "l" }))).toBe(true);

    expect(isUpKey(key({ name: "j" }))).toBe(false);
    expect(isDownKey(key({ name: "k" }))).toBe(false);
    expect(isLeftKey(key({ name: "l" }))).toBe(false);
    expect(isRightKey(key({ name: "h" }))).toBe(false);
  });

  it("detects confirm and escape key names", () => {
    expect(isConfirmKey(key({ name: "enter" }))).toBe(true);
    expect(isConfirmKey(key({ name: "return" }))).toBe(true);
    expect(isEscapeKey(key({ name: "escape" }))).toBe(true);
    expect(isEscapeKey(key({ name: "esc" }))).toBe(true);

    expect(isConfirmKey(key({ name: "space" }))).toBe(false);
    expect(isEscapeKey(key({ name: "enter" }))).toBe(false);
  });

  it("returns printable search characters", () => {
    expect(printableSearchChar(key({ name: "a" }))).toBe("a");
    expect(printableSearchChar(key({ name: "/" }))).toBe("/");
    expect(printableSearchChar(key({ name: "space" }))).toBe(" ");
    expect(printableSearchChar(key({ name: "unknown", raw: "Ж" }))).toBe(
      "Ж",
    );
  });

  it("excludes ctrl, meta, option, navigation, and control characters", () => {
    expect(printableSearchChar(key({ name: "a", ctrl: true }))).toBeNull();
    expect(printableSearchChar(key({ name: "a", meta: true }))).toBeNull();
    expect(printableSearchChar(key({ name: "a", option: true }))).toBeNull();
    expect(printableSearchChar(key({ name: "up" }))).toBeNull();
    expect(printableSearchChar(key({ name: "enter" }))).toBeNull();
    expect(printableSearchChar(key({ name: "tab" }))).toBeNull();
    expect(printableSearchChar(key({ name: "unknown", raw: "\x1B" }))).toBeNull();
  });
});
