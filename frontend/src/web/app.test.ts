import { access, readFile } from "node:fs/promises";
import { expect, test } from "vitest";

const webFile = (path: string) => new URL(path, import.meta.url);
const source = (path: string) => readFile(webFile(path), "utf8");

const relativeLuminance = (hex: string) => {
  const channels = [1, 3, 5].map((offset) =>
    Number.parseInt(hex.slice(offset, offset + 2), 16),
  );
  const [red, green, blue] = channels.map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
};

const contrastRatio = (foreground: string, background: string) => {
  const [lighter, darker] = [
    relativeLuminance(foreground),
    relativeLuminance(background),
  ].sort((left, right) => right - left);
  return (lighter + 0.05) / (darker + 0.05);
};

const hexToken = (theme: string, token: string) => {
  const alias = theme.match(
    new RegExp(`${token}:\\s*var\\((--thoth-[\\w-]+)\\)`),
  );
  const resolved = alias?.[1] ?? token;
  const match = theme.match(new RegExp(`${resolved}:\\s*(#[0-9a-f]{6})`, "i"));
  expect(match, `${token} should be a six-digit hex color`).not.toBeNull();
  return match![1];
};

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

test("accent foregrounds use a contrast-safe semantic text token", async () => {
  const css = await source("./app.css");
  expect(css).toContain("--color-accent-text: var(--hiero-accent-text)");

  const palette = await source("./theme/thoth.css");
  for (const name of ["light", "dark"]) {
    const block = new RegExp(
      `\\[data-theme="${name}"\\]\\s*\\{([\\s\\S]*?)\\n\\}`,
    );
    const theme = css.match(block)?.[1];
    const primitives = palette.match(block)?.[1];
    expect(theme).toBeDefined();
    expect(primitives).toBeDefined();
    const resolved = `${theme}\n${primitives}`;
    for (const foreground of [
      "--hiero-accent-text",
      "--hiero-text-primary",
      "--hiero-text-secondary",
      "--hiero-danger",
      "--hiero-success",
    ]) {
      for (const background of [
        "--hiero-bg-root",
        "--hiero-bg-surface",
        "--hiero-bg-raised",
      ]) {
        expect(
          contrastRatio(
            hexToken(resolved, foreground),
            hexToken(resolved, background),
          ),
          `${name}: ${foreground} on ${background}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  }

  for (const path of [
    "./components/AdminDashboard.svelte",
    "./components/DreamingEditor.svelte",
    "./components/IngestEditor.svelte",
    "./components/MemoryViews.svelte",
    "./components/ProviderEditor.svelte",
    "./components/ReleaseEditor.svelte",
  ]) {
    expect(await source(path)).not.toMatch(/\btext-accent(?!-text)\b/);
  }
});

test("the primary header wraps into accessible mobile navigation", async () => {
  const app = await source("./App.svelte");
  const headerContainer = app.match(
    /<header class="[^"]*">\s*<div class="([^"]*)"/,
  )?.[1];
  expect(headerContainer).toContain("flex-wrap");
  expect(headerContainer).toContain("sm:flex-nowrap");

  const navigation = app.match(/<nav class="([^"]*)"[^>]*>([\s\S]*?)<\/nav>/);
  expect(navigation).not.toBeNull();
  for (const utility of [
    "order-last",
    "w-full",
    "sm:order-none",
    "sm:w-auto",
    "sm:flex-1",
  ]) {
    expect(navigation![1]).toContain(utility);
  }

  const linkClasses = [...navigation![2].matchAll(/<a class="([^"]*)"/g)].map(
    (match) => match[1],
  );
  expect(linkClasses).toHaveLength(4);
  for (const classes of linkClasses) {
    expect(classes).toContain("inline-flex");
    expect(classes).toContain("min-h-11");
    expect(classes).toContain("items-center");
  }
});

test("all navigation links retain 44px interaction targets", async () => {
  const app = await source("./App.svelte");
  const navigationLinks = [...app.matchAll(/<nav\b[\s\S]*?<\/nav>/g)].flatMap(
    (navigation) =>
      [...navigation[0].matchAll(/<a class="([^"]*)"/g)].map((link) => link[1]),
  );
  expect(navigationLinks.length).toBeGreaterThan(3);
  for (const classes of navigationLinks) {
    expect(classes).toContain("inline-flex");
    expect(classes).toContain("min-h-11");
    expect(classes).toContain("items-center");
  }
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

test("the Tailwind stylesheet contains only used shared component selectors", async () => {
  const css = await source("./app.css");
  expect(css).not.toContain(".table-shell");
  expect(css).not.toContain(".toggle-track");
  expect(css).not.toContain(".toggle-thumb");
  expect(css).toContain(".data-table");
  expect(css).toContain(".editor-dialog");
});

test("inline error alerts use the semantic danger background", async () => {
  for (const path of [
    "./App.svelte",
    "./components/AdminDashboard.svelte",
    "./components/MemoryViews.svelte",
    "./components/DreamingEditor.svelte",
    "./components/IngestEditor.svelte",
    "./components/ProviderEditor.svelte",
    "./components/ReleaseEditor.svelte",
  ]) {
    const component = await source(path);
    const alerts =
      component.match(
        /<(?:p|div)[^>]*class="[^"]*border-danger[^"]*text-danger[^"]*"/g,
      ) ?? [];
    expect(
      alerts.length,
      `${path} should expose an inline error alert`,
    ).toBeGreaterThan(0);
    for (const alert of alerts) {
      expect(alert).toContain("bg-[var(--hiero-danger-bg)]");
      expect(alert).not.toContain("bg-raised");
    }
  }
});
