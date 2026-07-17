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
});

test("the web entry module imports the Tailwind stylesheet", async () => {
  const entryModule = await Bun.file(
    new URL("./main.ts", import.meta.url),
  ).text();

  expect(entryModule).toContain('import "./app.css";');
});
