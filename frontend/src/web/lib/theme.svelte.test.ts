import { beforeEach, expect, test, vi } from "vitest";

beforeEach(() => {
  vi.resetModules();
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: false })),
  );
});

test("theme toggle applies and persists the selected theme", async () => {
  const { createThemeToggle } = await import("./theme.svelte");
  const theme = createThemeToggle();

  expect(theme.theme).toBe("dark");
  expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

  theme.toggle();

  expect(theme.theme).toBe("light");
  expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  expect(localStorage.getItem("hiero-theme")).toBe("light");
});
