import React from "react";

type InlineToken =
  | { kind: "text"; value: string }
  | { kind: "bold"; value: string }
  | { kind: "italic"; value: string }
  | { kind: "code"; value: string }
  | { kind: "link"; label: string; url: string };

const inlinePattern =
  /(`([^`]+)`|\[([^\]]+)\]\(([^)\s]+)(?:\s+"[^"]*")?\)|\*\*([^*]+)\*\*|\*([^*]+)\*)/g;

function parseInline(text: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  let offset = 0;
  let match: RegExpExecArray | null;

  while ((match = inlinePattern.exec(text)) !== null) {
    if (match.index > offset) {
      tokens.push({ kind: "text", value: text.slice(offset, match.index) });
    }

    if (match[2] !== undefined) {
      tokens.push({ kind: "code", value: match[2] });
    } else if (match[3] !== undefined && match[4] !== undefined) {
      tokens.push({ kind: "link", label: match[3], url: match[4] });
    } else if (match[5] !== undefined) {
      tokens.push({ kind: "bold", value: match[5] });
    } else {
      tokens.push({ kind: "italic", value: match[6] ?? "" });
    }

    offset = match.index + match[0].length;
  }

  if (offset < text.length) {
    tokens.push({ kind: "text", value: text.slice(offset) });
  }

  return tokens;
}

function renderInline(text: string, keyPrefix: string): React.ReactNode[] {
  return parseInline(text).map((token, index) => {
    const key = `${keyPrefix}-${index}`;
    if (token.kind === "bold") {
      return <strong key={key}>{token.value}</strong>;
    }
    if (token.kind === "italic") {
      return <em key={key}>{token.value}</em>;
    }
    if (token.kind === "code") {
      return <React.Fragment key={key}>`{token.value}`</React.Fragment>;
    }
    if (token.kind === "link") {
      return (
        <React.Fragment key={key}>
          {token.label} ({token.url})
        </React.Fragment>
      );
    }
    return <React.Fragment key={key}>{token.value}</React.Fragment>;
  });
}

function isHorizontalRule(line: string): boolean {
  const trimmed = line.trim();
  return trimmed.length >= 3 && /^([-*_])(?:\s*\1){2,}$/.test(trimmed);
}

function isBlockStart(line: string): boolean {
  return (
    line.trim() === "" ||
    /^\s{0,3}(```|~~~)/.test(line) ||
    /^\s{0,3}#{1,6}\s+/.test(line) ||
    /^\s{0,3}([-*+])\s+/.test(line) ||
    /^\s{0,3}\d+[.)]\s+/.test(line) ||
    /^\s{0,3}>\s?/.test(line) ||
    isHorizontalRule(line)
  );
}

export function MarkdownBody({ content }: { content: string }) {
  const lines = content.split("\n");
  const blocks: React.ReactNode[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index] ?? "";
    const key = `markdown-${index}`;

    if (line.trim() === "") {
      blocks.push(<text key={key}> </text>);
      index += 1;
      continue;
    }

    const fence = line.match(/^\s{0,3}(```|~~~)\s*([A-Za-z0-9_-]*)\s*$/);
    if (fence) {
      const marker = fence[1];
      const language = fence[2] || "text";
      const codeLines: string[] = [];
      index += 1;
      while (index < lines.length) {
        const codeLine = lines[index] ?? "";
        if (new RegExp(`^\\s{0,3}${marker}\\s*$`).test(codeLine)) {
          index += 1;
          break;
        }
        codeLines.push(codeLine);
        index += 1;
      }
      blocks.push(
        <box key={key} flexDirection="column">
          {language === "text" ? null : <text>```{language}</text>}
          {codeLines.length === 0 ? (
            <text> </text>
          ) : (
            codeLines.map((codeLine, codeIndex) => (
              <text key={`${key}-code-${codeIndex}`}>{codeLine || " "}</text>
            ))
          )}
        </box>,
      );
      continue;
    }

    const heading = line.match(/^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$/);
    if (heading) {
      blocks.push(
        <text key={key}>
          <strong>{renderInline(heading[1], `${key}-heading`)}</strong>
        </text>,
      );
      index += 1;
      continue;
    }

    const unordered = line.match(/^\s{0,3}[-*+]\s+(.+)$/);
    if (unordered) {
      blocks.push(
        <text key={key}>- {renderInline(unordered[1], `${key}-ul`)}</text>,
      );
      index += 1;
      continue;
    }

    const ordered = line.match(/^\s{0,3}(\d+)[.)]\s+(.+)$/);
    if (ordered) {
      blocks.push(
        <text key={key}>
          {ordered[1]}. {renderInline(ordered[2], `${key}-ol`)}
        </text>,
      );
      index += 1;
      continue;
    }

    const quote = line.match(/^\s{0,3}>\s?(.*)$/);
    if (quote) {
      blocks.push(
        <text key={key}>| {renderInline(quote[1], `${key}-quote`)}</text>,
      );
      index += 1;
      continue;
    }

    if (isHorizontalRule(line)) {
      blocks.push(<text key={key}>----------------</text>);
      index += 1;
      continue;
    }

    blocks.push(
      <text key={key}>{renderInline(line.trim(), `${key}-paragraph`)}</text>,
    );
    index += 1;
  }

  return <box flexDirection="column">{blocks}</box>;
}
