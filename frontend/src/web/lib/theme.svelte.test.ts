import { expect, test } from "bun:test";

test("theme toggle applies and persists the selected theme", async () => {
  const attributes = new Map<string, string>();
  const storage = new Map<string, string>();

  Object.assign(globalThis, {
    $state<T>(value: T) {
      return value;
    },
    document: {
      documentElement: {
        setAttribute(name: string, value: string) {
          attributes.set(name, value);
        },
      },
    },
    localStorage: {
      getItem(key: string) {
        return storage.get(key) ?? null;
      },
      setItem(key: string, value: string) {
        storage.set(key, value);
      },
    },
    window: {
      matchMedia() {
        return { matches: false };
      },
    },
  });

  const { createThemeToggle } = await import("./theme.svelte");
  const theme = createThemeToggle();

  expect(theme.theme).toBe("dark");
  expect(attributes.get("data-theme")).toBe("dark");

  theme.toggle();

  expect(theme.theme).toBe("light");
  expect(attributes.get("data-theme")).toBe("light");
  expect(storage.get("hiero-theme")).toBe("light");
});
