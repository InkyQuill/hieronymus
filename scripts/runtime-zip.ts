/** Bounded ZIP32 reader. Reject encryption, ZIP64, links, ambiguous headers and paths. */
import { inflateRawSync } from "node:zlib";
export function crc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const b of bytes) {
    crc ^= b;
    for (let j = 0; j < 8; j++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
export function zipMembers(
  bytes: Buffer,
  limit: number,
): Map<string, Buffer | null> {
  if (bytes.length < 22) throw new Error("truncated ZIP");
  let end = bytes.length - 22;
  while (
    end >= Math.max(0, bytes.length - 65557) &&
    bytes.readUInt32LE(end) !== 0x06054b50
  )
    end--;
  if (
    end < 0 ||
    end + 22 + bytes.readUInt16LE(end + 20) !== bytes.length ||
    bytes.readUInt16LE(end + 4) ||
    bytes.readUInt16LE(end + 6)
  )
    throw new Error("invalid ZIP terminator");
  const count = bytes.readUInt16LE(end + 10),
    size = bytes.readUInt32LE(end + 12),
    start = bytes.readUInt32LE(end + 16);
  if (
    count > 10000 ||
    count !== bytes.readUInt16LE(end + 8) ||
    start + size !== end
  )
    throw new Error("unsupported ZIP directory");
  const result = new Map<string, Buffer | null>();
  let offset = start,
    expanded = 0,
    localEnd = 0;
  for (let n = 0; n < count; n++) {
    if (offset + 46 > end || bytes.readUInt32LE(offset) !== 0x02014b50)
      throw new Error("invalid ZIP central entry");
    const flags = bytes.readUInt16LE(offset + 8),
      method = bytes.readUInt16LE(offset + 10),
      crc = bytes.readUInt32LE(offset + 16),
      compressed = bytes.readUInt32LE(offset + 20),
      length = bytes.readUInt32LE(offset + 24),
      namesize = bytes.readUInt16LE(offset + 28),
      extra = bytes.readUInt16LE(offset + 30),
      comment = bytes.readUInt16LE(offset + 32),
      external = bytes.readUInt32LE(offset + 38),
      local = bytes.readUInt32LE(offset + 42);
    if (
      offset + 46 + namesize + extra + comment > end ||
      flags & ~0x800 ||
      ![0, 8].includes(method) ||
      bytes.readUInt16LE(offset + 34) ||
      external >>> 28 === 10 ||
      (external >>> 28 && ![4, 8].includes(external >>> 28))
    )
      throw new Error("unsupported ZIP entry");
    const raw = bytes.subarray(offset + 46, offset + 46 + namesize),
      name = raw.toString("utf8"),
      directory = name.endsWith("/"),
      key = directory ? name.slice(0, -1) : name;
    if (
      !Buffer.from(name).equals(raw) ||
      !key ||
      key.includes("\\") ||
      key
        .split("/")
        .some(
          (p) => !p || p === "." || p === ".." || /[:\x00-\x1f]|[. ]$/.test(p),
        ) ||
      result.has(key)
    )
      throw new Error("unsafe or duplicate ZIP path");
    if (
      local !== localEnd ||
      local + 30 > start ||
      bytes.readUInt32LE(local) !== 0x04034b50 ||
      bytes.readUInt16LE(local + 6) !== flags ||
      bytes.readUInt16LE(local + 8) !== method ||
      bytes.readUInt32LE(local + 14) !== crc ||
      bytes.readUInt32LE(local + 18) !== compressed ||
      bytes.readUInt32LE(local + 22) !== length ||
      bytes.readUInt16LE(local + 26) !== namesize
    )
      throw new Error("ZIP local header mismatch");
    const dataStart = local + 30 + namesize + bytes.readUInt16LE(local + 28);
    localEnd = dataStart + compressed;
    if (
      localEnd > start ||
      !bytes.subarray(local + 30, local + 30 + namesize).equals(raw) ||
      (expanded += length) > limit ||
      (directory && length)
    )
      throw new Error("ZIP expanded bound or header mismatch");
    const data = bytes.subarray(dataStart, localEnd);
    const output =
      method === 0
        ? data
        : inflateRawSync(data, {
            maxOutputLength: Math.max(1, Math.min(length, limit)),
          });
    if (output.length !== length || crc32(output) !== crc)
      throw new Error("ZIP size or CRC mismatch");
    result.set(key, directory ? null : output);
    offset += 46 + namesize + extra + comment;
  }
  if (offset !== end || localEnd !== start)
    throw new Error("unaccounted ZIP bytes");
  for (const name of result.keys()) {
    const parts = name.split("/");
    parts.pop();
    while (parts.length) {
      const parent = parts.join("/");
      if (result.has(parent) && result.get(parent) !== null)
        throw new Error("ZIP parent is not a directory");
      parts.pop();
    }
  }
  return result;
}
