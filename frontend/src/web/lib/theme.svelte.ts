type Theme = "dark" | "light";

const STORAGE_KEY = "hiero-theme";

function systemPreference(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

function storedTheme(): Theme | null {
  if (typeof localStorage === "undefined") return null;
  const value = localStorage.getItem(STORAGE_KEY);
  return value === "dark" || value === "light" ? value : null;
}

function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute("data-theme", theme);
}

let current = $state<Theme>(storedTheme() ?? systemPreference());

function applyCurrentTheme(): void {
  applyTheme(current);
}

applyCurrentTheme();

export function createThemeToggle() {
  function toggle(): void {
    current = current === "dark" ? "light" : "dark";
    localStorage.setItem(STORAGE_KEY, current);
    applyCurrentTheme();
  }

  return {
    get theme(): Theme {
      return current;
    },
    toggle,
  };
}
