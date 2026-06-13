export type TerminalLayoutKind = "wide" | "compact" | "narrow" | "too-small";

export type TerminalLayout = {
  kind: TerminalLayoutKind;
  width: number;
  height: number;
};

export const MIN_TERMINAL_WIDTH = 50;
export const MIN_TERMINAL_HEIGHT = 20;
export const MIN_COMPACT_WIDTH = 80;
export const MIN_COMPACT_HEIGHT = 24;
export const WIDE_WIDTH = 132;

export function classifyTerminalLayout(
  width: number,
  height: number,
): TerminalLayout {
  if (width < MIN_TERMINAL_WIDTH || height < MIN_TERMINAL_HEIGHT) {
    return { kind: "too-small", width, height };
  }

  if (width >= WIDE_WIDTH && height >= MIN_COMPACT_HEIGHT) {
    return { kind: "wide", width, height };
  }

  if (width >= MIN_COMPACT_WIDTH && height >= MIN_COMPACT_HEIGHT) {
    return { kind: "compact", width, height };
  }

  return { kind: "narrow", width, height };
}

export function panelWidth(layout: TerminalLayout, borderPadding = 2): number {
  return Math.max(20, layout.width - borderPadding * 2);
}

export function panelHeight(
  layout: TerminalLayout,
  reservedRows: number,
): number {
  return Math.max(4, layout.height - reservedRows);
}
