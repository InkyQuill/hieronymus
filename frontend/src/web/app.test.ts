import { expect, test } from "bun:test";

test("the web stylesheet defines the data-theme Tailwind dark variant", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();
  expect(css).toContain(
    '@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));',
  );
  expect(css).toContain('@import "tailwindcss";');
});

test("the semantic theme preserves both runtime modes and editorial utilities", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();
  for (const token of [
    "--hiero-bg-root",
    "--hiero-text-primary",
    "--hiero-danger",
    "--hiero-success",
  ]) {
    expect(css).toContain(token);
  }
  expect(css).toContain("@utility text-display");
  expect(css).toContain("--animate-toast-in");
  expect(css).toContain("--breakpoint-sm: 45rem");
  expect(css).toMatch(/\[data-theme="light"\]\s*\{[\s\S]*?--hiero-bg-root:/);
  expect(css).toMatch(/\[data-theme="dark"\]\s*\{[\s\S]*?--hiero-bg-root:/);
});

test("the editor dialog remains within narrow viewports while right-aligned", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();

  expect(css).toMatch(
    /\.editor-dialog\s*\{[\s\S]*?right:\s*0;[\s\S]*?width:\s*min\(420px,\s*100%\);/,
  );
});

test("the web entry module imports the Tailwind stylesheet", async () => {
  const entryModule = await Bun.file(
    new URL("./main.ts", import.meta.url),
  ).text();

  expect(entryModule).toContain('import "./app.css";');
});
