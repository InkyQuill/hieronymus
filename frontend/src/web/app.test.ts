import { expect, test } from "bun:test";

test("the app shell uses the semantic Tailwind surface utilities", async () => {
  const app = await Bun.file(new URL("./App.svelte", import.meta.url)).text();

  expect(app).toContain("min-h-dvh");
  expect(app).toContain("bg-root");
  expect(app).toContain("border-default");
});

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
  expect(css).toContain("--color-strong: var(--hiero-border-strong)");
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

test("the legacy stylesheets have been retired", async () => {
  for (const filename of ["tokens.css", "base.css", "components.css"]) {
    expect(
      await Bun.file(new URL(`./${filename}`, import.meta.url)).exists(),
    ).toBe(false);
  }
});

test("memory rows keep keyboard activation after Tailwind migration", async () => {
  const memoryView = await Bun.file(
    new URL("./components/MemoryViews.svelte", import.meta.url),
  ).text();

  expect(memoryView).toContain('event.key === "Enter"');
  expect(memoryView).toContain("event.preventDefault()");
  expect(memoryView).toContain("overflow-x-auto");
});

test("editor controls preserve native dialog and accessible toggle markup", async () => {
  const providerEditor = await Bun.file(
    new URL("./components/ProviderEditor.svelte", import.meta.url),
  ).text();
  const dreamingEditor = await Bun.file(
    new URL("./components/DreamingEditor.svelte", import.meta.url),
  ).text();

  expect(providerEditor).toContain("<dialog");
  expect(providerEditor).toContain("aria-labelledby");
  expect(dreamingEditor).toContain("peer");
  expect(dreamingEditor).toContain("min-h-11");
  expect(dreamingEditor).toContain("peer-checked:[&>span]:translate-x-[18px]");
  expect(dreamingEditor).toContain("peer-checked:[&>span]:bg-accent");
});
