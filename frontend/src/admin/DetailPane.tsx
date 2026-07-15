import React from "react";
import { SyntaxStyle } from "@opentui/core";
import type { AdminSnapshot } from "../rpc/schema.js";
import { MarkdownBody } from "./markdown.js";

const codeSyntaxStyle = SyntaxStyle.fromStyles({
  string: { fg: "#9ece6a" },
  number: { fg: "#ff9e64" },
  boolean: { fg: "#bb9af7" },
  property: { fg: "#7dcfff" },
  punctuation: { fg: "#a9b1d6" },
  comment: { fg: "#565f89", italic: true },
  keyword: { fg: "#bb9af7", bold: true },
});

const isDiff = (text: string) => {
  return (
    text.startsWith("diff ") ||
    text.startsWith("Index: ") ||
    text.includes("\n--- ") ||
    text.includes("\n+++ ") ||
    text.includes("\n@@ -")
  );
};

const isJson = (text: string) => {
  const trimmed = text.trim();
  if (
    !(
      (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
      (trimmed.startsWith("[") && trimmed.endsWith("]"))
    )
  ) {
    return false;
  }
  try {
    JSON.parse(trimmed);
    return true;
  } catch {
    return false;
  }
};

export function DetailPane({
  detail,
  width = 56,
  height = 14,
}: {
  detail: AdminSnapshot["detail"];
  width?: number;
  height?: number;
}) {
  const renderBody = () => {
    if (!detail.body) return null;
    if (isDiff(detail.body)) {
      return (
        <diff
          diff={detail.body}
          filetype="diff"
          syntaxStyle={codeSyntaxStyle}
        />
      );
    }
    if (isJson(detail.body)) {
      return (
        <code
          content={detail.body}
          filetype="json"
          syntaxStyle={codeSyntaxStyle}
        />
      );
    }
    return <MarkdownBody content={detail.body} />;
  };

  return (
    <scrollbox
      key={height < 3 ? "scrollbar-hidden" : "scrollbar-auto"}
      width={width}
      height={height}
      style={{
        verticalScrollbarOptions: {
          showArrows: height >= 3,
          ...(height < 3 ? { visible: false } : {}),
          width: 1,
          position: "absolute",
          right: 0,
          top: 0,
          bottom: 0,
        },
      }}
    >
      <text flexShrink={0}>{detail.title}</text>
      <text flexShrink={0} fg="gray">
        {detail.subtitle}
      </text>
      <box flexDirection="column" flexShrink={0} marginTop={1} marginBottom={1}>
        {renderBody()}
      </box>
      {detail.fields.map(([name, value], index) => (
        <text key={`${name}-${index}`} flexShrink={0}>
          {name}: {value}
        </text>
      ))}
    </scrollbox>
  );
}
