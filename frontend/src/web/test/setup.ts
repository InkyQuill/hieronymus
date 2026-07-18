import { cleanup } from "@testing-library/svelte";
import { afterEach, vi } from "vitest";

const storedValues = new Map<string, string>();
const browserStorage: Storage = {
  get length() {
    return storedValues.size;
  },
  clear: () => storedValues.clear(),
  getItem: (key) => storedValues.get(key) ?? null,
  key: (index) => [...storedValues.keys()][index] ?? null,
  removeItem: (key) => storedValues.delete(key),
  setItem: (key, value) => storedValues.set(key, String(value)),
};

const installBrowserStorage = () => {
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: browserStorage,
  });
};

installBrowserStorage();

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  vi.unstubAllGlobals();
  installBrowserStorage();
});
