import { access, readFile } from "node:fs/promises";
import { expect, test } from "vitest";

const webFile = (path: string) => new URL(path, import.meta.url);
const source = (path: string) => readFile(webFile(path), "utf8");

test("the web stylesheet configures Tailwind and the data-theme dark variant", async () => {
  const css = await source("./app.css");
  expect(css).toContain('@import "tailwindcss";');
  expect(css).toContain(
    '@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));',
  );
});

test("the semantic theme exposes required runtime tokens and utilities", async () => {
  const css = await source("./app.css");
  for (const token of [
    "--hiero-bg-root",
    "--hiero-text-primary",
    "--hiero-danger",
    "--hiero-success",
  ]) {
    expect(css).toContain(token);
  }
  expect(css).toContain("@utility text-display");
  expect(css).toContain("--color-strong: var(--hiero-border-strong)");
  expect(css).toContain("--animate-toast-in");
  expect(css).toContain("--breakpoint-sm: 45rem");
  expect(css).toMatch(/\[data-theme="light"\]\s*\{[\s\S]*?--hiero-bg-root:/);
  expect(css).toMatch(/\[data-theme="dark"\]\s*\{[\s\S]*?--hiero-bg-root:/);
});

test("the editor dialog source contract remains viewport-bounded and right-aligned", async () => {
  const css = await source("./app.css");
  expect(css).toMatch(
    /\.editor-dialog\s*\{[\s\S]*?right:\s*0;[\s\S]*?width:\s*min\(420px,\s*100%\);/,
  );
});

test("the web entry imports the Tailwind stylesheet", async () => {
  expect(await source("./main.ts")).toContain('import "./app.css";');
});

test("legacy stylesheets remain deleted", async () => {
  await expect(access(webFile("./base.css"))).rejects.toThrow();
  await expect(access(webFile("./tokens.css"))).rejects.toThrow();
  await expect(access(webFile("./components.css"))).rejects.toThrow();
});
