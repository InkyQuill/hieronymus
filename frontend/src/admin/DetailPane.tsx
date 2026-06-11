import React from "react";
import { SyntaxStyle } from "@opentui/core";
import type { AdminSnapshot } from "../rpc/schema.js";

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

const renderInlineMarkdown = (text: string, keyPrefix: string) => {
  const parts: React.ReactNode[] = [];
  const pattern = /(<strong>(.*?)<\/strong>|\*\*(.*?)\*\*)/gi;
  let offset = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > offset) {
      parts.push(text.slice(offset, match.index));
    }
    parts.push(
      <strong key={`${keyPrefix}-${match.index}`}>
        {match[2] ?? match[3] ?? ""}
      </strong>,
    );
    offset = match.index + match[0].length;
  }
  if (offset < text.length) {
    parts.push(text.slice(offset));
  }
  return parts;
};

const renderMarkdownLines = (text: string) => {
  return text.split("\n").map((line, index) => {
    const displayLine = line
      .replace(/^\s{0,3}#{1,6}\s+/, "")
      .replace(/^\s{0,3}[-*+]\s+/, "- ");
    return (
      <text key={`${index}-${displayLine}`}>
        {renderInlineMarkdown(displayLine, `line-${index}`)}
      </text>
    );
  });
};

export function DetailPane({ detail }: { detail: AdminSnapshot["detail"] }) {
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
    return <box flexDirection="column">{renderMarkdownLines(detail.body)}</box>;
  };

  return (
    <scrollbox flexDirection="column" width={56} height={14}>
      <text>{detail.title}</text>
      <text fg="gray">{detail.subtitle}</text>
      <box flexDirection="column" marginTop={1} marginBottom={1}>
        {renderBody()}
      </box>
      {detail.fields.map(([name, value], index) => (
        <text key={`${name}-${index}`}>
          {name}: {value}
        </text>
      ))}
    </scrollbox>
  );
}
