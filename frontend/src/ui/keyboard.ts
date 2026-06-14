export type KeyboardInput = {
  name: string;
  ctrl?: boolean;
  meta?: boolean;
  shift?: boolean;
  option?: boolean;
  raw?: string;
  sequence?: string;
};

const NAVIGATION_KEYS = new Set([
  "up",
  "down",
  "left",
  "right",
  "tab",
  "enter",
  "return",
  "escape",
  "esc",
  "backspace",
  "delete",
  "home",
  "end",
  "pageup",
  "pagedown",
]);

export function isUpKey(key: KeyboardInput): boolean {
  return key.name === "up" || key.name === "k";
}

export function isDownKey(key: KeyboardInput): boolean {
  return key.name === "down" || key.name === "j";
}

export function isLeftKey(key: KeyboardInput): boolean {
  return key.name === "left" || key.name === "h";
}

export function isRightKey(key: KeyboardInput): boolean {
  return key.name === "right" || key.name === "l";
}

export function isConfirmKey(key: KeyboardInput): boolean {
  return key.name === "enter" || key.name === "return";
}

export function isEscapeKey(key: KeyboardInput): boolean {
  return key.name === "escape" || key.name === "esc";
}

export function printableSearchChar(key: KeyboardInput): string | null {
  if (key.ctrl || key.meta || key.option || NAVIGATION_KEYS.has(key.name)) {
    return null;
  }

  if (key.name === "space") {
    return " ";
  }

  if (key.name.length === 1) {
    return key.name;
  }

  if (key.raw?.length === 1 && !isControlCharacter(key.raw)) {
    return key.raw;
  }

  return null;
}

function isControlCharacter(value: string): boolean {
  const code = value.charCodeAt(0);
  return code < 32 || code === 127;
}
