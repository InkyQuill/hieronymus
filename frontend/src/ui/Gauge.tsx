import React from "react";

const FILLED_BLOCK = "█";
const EMPTY_BLOCK = "░";
const DEFAULT_BAR_WIDTH = 10;
const MIN_BAR_RENDER_WIDTH = 16;

export type GaugeProps = {
  label: string;
  value: number;
  max: number;
  width?: number;
  barWidth?: number;
  fg?: string;
};

export function Gauge({
  label,
  value,
  max,
  width,
  barWidth = DEFAULT_BAR_WIDTH,
  fg,
}: GaugeProps) {
  const safeMax = Math.max(max, 0);
  const hasBarSpace = width === undefined || width >= MIN_BAR_RENDER_WIDTH;

  if (safeMax === 0 || !hasBarSpace) {
    return (
      <text fg={fg}>
        {label} {value}/{safeMax}
      </text>
    );
  }

  const ratio = Math.min(Math.max(value / safeMax, 0), 1);
  const filled = Math.round(ratio * barWidth);
  const bar =
    FILLED_BLOCK.repeat(filled) + EMPTY_BLOCK.repeat(barWidth - filled);

  return (
    <text fg={fg}>
      {label} [{bar}] {value}/{safeMax}
    </text>
  );
}
