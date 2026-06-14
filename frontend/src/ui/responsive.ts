export type TerminalLayoutKind = "wide" | "compact" | "narrow" | "too-small";

export type TerminalLayout = {
  kind: TerminalLayoutKind;
  width: number;
  height: number;
};

export const MIN_TERMINAL_WIDTH = 60;
export const MIN_TERMINAL_HEIGHT = 20;
export const MIN_COMPACT_WIDTH = 80;
export const MIN_COMPACT_HEIGHT = 24;
export const WIDE_WIDTH = 136;
export const MIN_PANEL_WIDTH = 20;
export const MIN_PANEL_HEIGHT = 4;

/**
 * Classifies terminal dimensions for OpenTUI screen layouts.
 *
 * `too-small` is returned for invalid dimensions or anything below
 * MIN_TERMINAL_WIDTH x MIN_TERMINAL_HEIGHT. `wide` starts at WIDE_WIDTH and
 * MIN_COMPACT_HEIGHT. `compact` starts at MIN_COMPACT_WIDTH x
 * MIN_COMPACT_HEIGHT. The final `narrow` branch is a size-limited fallback for
 * terminals that meet the minimum floor but cannot use compact or wide layouts.
 */
export function classifyTerminalLayout(
  width: number,
  height: number,
): TerminalLayout {
  if (!Number.isFinite(width) || !Number.isFinite(height)) {
    return { kind: "too-small", width, height };
  }

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
  return Math.max(MIN_PANEL_WIDTH, layout.width - borderPadding * 2);
}

export function panelHeight(
  layout: TerminalLayout,
  reservedRows: number,
): number {
  return Math.max(MIN_PANEL_HEIGHT, layout.height - reservedRows);
}
