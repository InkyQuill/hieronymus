import { CliRenderEvents, type ScrollBoxRenderable } from "@opentui/core";
import { useRenderer } from "@opentui/react";
import React, { useCallback, useEffect, useRef } from "react";
import stringWidth from "string-width";
import type { AdminRow } from "../rpc/schema.js";

const MARKER_WIDTH = 2;
const COLUMN_GAP = 1;
const STATUS_COLUMN_WIDTH = 10;
const QUALITY_COLUMN_WIDTH = 10;
const MIN_LABEL_COLUMN_WIDTH = 4;
const MIN_COLUMN_LAYOUT_WIDTH = 4;
const ELLIPSIS = "…";
const ELLIPSIS_WIDTH = stringWidth(ELLIPSIS);
const GRAPHEME_SEGMENTER = new Intl.Segmenter(undefined, {
  granularity: "grapheme",
});

export function AdminTable({
  rows,
  selectedId,
  focused = true,
  width = 48,
  height = 18,
}: {
  rows: AdminRow[];
  selectedId: string | number | null;
  focused?: boolean;
  width?: number;
  height?: number;
}) {
  const renderer = useRenderer();
  const scrollboxRef = useRef<ScrollBoxRenderable>(null);
  const layout = width >= MIN_COLUMN_LAYOUT_WIDTH ? columnWidths(width) : null;
  const selectedIndex = rows.findIndex((row) => row.id === selectedId);
  const scrollbarMode = height < 3 ? "hidden" : "auto";
  const scrollSelectedRowIntoView = useCallback(() => {
    if (selectedIndex >= 0) {
      scrollboxRef.current?.scrollChildIntoView(rowId(selectedIndex));
    }
  }, [selectedIndex]);

  useEffect(() => {
    if (selectedIndex < 0) {
      return;
    }

    renderer.once(CliRenderEvents.FRAME, scrollSelectedRowIntoView);
    return () => {
      renderer.off(CliRenderEvents.FRAME, scrollSelectedRowIntoView);
    };
  }, [renderer, scrollSelectedRowIntoView, scrollbarMode, selectedIndex]);

  return (
    <scrollbox
      key={scrollbarMode}
      ref={scrollboxRef}
      width={width}
      height={height}
      style={{
        verticalScrollbarOptions: {
          showArrows: scrollbarMode === "auto",
          ...(scrollbarMode === "hidden" ? { visible: false } : {}),
          width: 1,
          position: "absolute",
          right: 0,
          top: 0,
          bottom: 0,
        },
      }}
    >
      {rows.map((row, index) => (
        <text
          key={String(row.id)}
          id={rowId(index)}
          fg={row.id === selectedId ? (focused ? "cyan" : "white") : undefined}
        >
          {layout === null ? (
            tinyRow(row.label, row.id === selectedId, width)
          ) : (
            <>
              {row.id === selectedId ? "> " : "  "}
              {padColumn(row.label, layout.labelWidth)}{" "}
              {padColumn(row.status, layout.statusWidth)}{" "}
              {padColumn(row.quality_label, layout.qualityWidth)}
            </>
          )}
        </text>
      ))}
    </scrollbox>
  );
}

function rowId(index: number): string {
  return `admin-table-row-${index}`;
}

function tinyRow(label: string, selected: boolean, width: number): string {
  if (width <= 0) {
    return "";
  }
  return `${selected ? ">" : " "}${padColumn(label, width - 1)}`;
}

function columnWidths(width: number): {
  labelWidth: number;
  statusWidth: number;
  qualityWidth: number;
} {
  const columnSpace = Math.max(0, width - MARKER_WIDTH - COLUMN_GAP * 2);
  const metadataSpace = Math.min(
    STATUS_COLUMN_WIDTH + QUALITY_COLUMN_WIDTH,
    Math.max(0, columnSpace - MIN_LABEL_COLUMN_WIDTH),
  );
  const statusWidth = Math.min(
    STATUS_COLUMN_WIDTH,
    Math.ceil(metadataSpace / 2),
  );
  const qualityWidth = Math.min(
    QUALITY_COLUMN_WIDTH,
    metadataSpace - statusWidth,
  );

  return {
    labelWidth: columnSpace - statusWidth - qualityWidth,
    statusWidth,
    qualityWidth,
  };
}

function padColumn(value: string, columnWidth: number): string {
  if (columnWidth <= 0) {
    return "";
  }

  const valueWidth = stringWidth(value);
  if (valueWidth <= columnWidth) {
    return `${value}${" ".repeat(columnWidth - valueWidth)}`;
  }

  const content = truncateToWidth(value, columnWidth);
  return `${content}${" ".repeat(columnWidth - stringWidth(content))}`;
}

function truncateToWidth(value: string, width: number): string {
  if (width <= ELLIPSIS_WIDTH) {
    return ELLIPSIS;
  }

  const contentWidth = width - ELLIPSIS_WIDTH;
  let content = "";
  let usedWidth = 0;

  for (const { segment } of GRAPHEME_SEGMENTER.segment(value)) {
    const segmentWidth = stringWidth(segment);
    if (usedWidth + segmentWidth > contentWidth) {
      break;
    }
    content += segment;
    usedWidth += segmentWidth;
  }

  return `${content}${ELLIPSIS}`;
}
