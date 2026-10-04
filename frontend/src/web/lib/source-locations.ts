/** Compact the display only; the snapshot keeps every original source reference. */
export function compactSourceLocations(value: string): string[] {
  const groups = new Map<string, { path: string; context: string; spans: [number, number][] }>();
  const entries: Array<string | { path: string; context: string; spans: [number, number][] }> = [];
  for (const reference of value.split("; ")) {
    const match = /^(.*\.[^/\\:;\s]+):([\d,\s-]+)(;sha256=[a-fA-F0-9]{64})?(.*)$/.exec(reference);
    if (!match) { entries.push(reference); continue; }
    const [, path, lines, checksum = "", context] = match;
    const spans: [number, number][] = [];
    let valid = true;
    for (const range of lines.split(",")) {
      const parsed = /^(\d+)(?:-(\d+))?$/.exec(range.trim());
      if (!parsed) { valid = false; break; }
      const start = Number(parsed[1]), end = Number(parsed[2] ?? parsed[1]);
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 1 || end < start) { valid = false; break; }
      spans.push([start, end]);
    }
    if (!valid) { entries.push(reference); continue; }
    const key = JSON.stringify([path, checksum.toLowerCase(), context]);
    let group = groups.get(key);
    if (!group) {
      group = { path, context, spans: [] };
      groups.set(key, group); entries.push(group);
    }
    group.spans.push(...spans);
  }
  return entries.map(entry => {
    if (typeof entry === "string") return entry;
    const merged: [number, number][] = [];
    for (const [start, end] of entry.spans.sort((a, b) => a[0] - b[0])) {
      const previous = merged.at(-1);
      if (previous && start <= previous[1] + 1) previous[1] = Math.max(previous[1], end);
      else merged.push([start, end]);
    }
    const lines = merged.map(([start, end]) => start === end ? String(start) : `${start}–${end}`).join(", ");
    return `${entry.path}:${lines}${entry.context}`;
  });
}
