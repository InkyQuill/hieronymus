import { expect, test } from "vitest";
import { compactSourceLocations } from "./source-locations";

const hash = "a".repeat(64);
const source = (line: string | number, checksum = hash, context = "") => `/book/decisions.md:${line};sha256=${checksum}${context}`;

test("consecutive citations show one file and a range without losing the original references", () => {
  const original = Array.from({length: 19}, (_, index) => source(index + 142)).join("; ");
  expect(compactSourceLocations(original)).toEqual(["/book/decisions.md:142–160"]);
  expect(original).toContain(source(150));
});
test("gaps, duplicate lines and overlapping ranges preserve the exact cited set", () => {
  expect(compactSourceLocations([source(145), source("142-144"), source(144), source(160), source(162)].join("; "))).toEqual(["/book/decisions.md:142–145, 160, 162"]);
});
test("different revisions and chapter contexts are never merged", () => {
  expect(compactSourceLocations([source(142), source(143,"b".repeat(64)), source(144, hash," · Chapter 2")].join("; "))).toEqual(["/book/decisions.md:142", "/book/decisions.md:143", "/book/decisions.md:144 · Chapter 2"]);
});
test("plain locations, Windows paths and malformed ranges remain readable", () => {
  expect(compactSourceLocations("Vol 3, chapter 2; C:\\book\\notes.md:12; C:\\book\\notes.md:13; /book/notes.md:5-2")).toEqual(["Vol 3, chapter 2", "C:\\book\\notes.md:12–13", "/book/notes.md:5-2"]);
});

test("compact agent references keep disjoint ranges in a single displayed source", () => {
  expect(compactSourceLocations(source("334,351-353,366,369"))).toEqual(["/book/decisions.md:334, 351–353, 366, 369"]);
});
