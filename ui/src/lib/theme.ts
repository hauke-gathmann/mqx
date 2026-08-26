export type Theme = "dark" | "light" | "system";

const STORAGE_KEY = "mqx:theme";

export const THEMES: Theme[] = ["dark", "light", "system"];

export function isTheme(value: unknown): value is Theme {
  return value === "dark" || value === "light" || value === "system";
}

export function readCachedTheme(): Theme {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    if (isTheme(value)) {
      return value;
    }
  } catch {
    // Private mode should still boot with the default theme.
  }
  return "dark";
}

export function cacheTheme(theme: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Ignore quota / private-mode failures.
  }
}

export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  cacheTheme(theme);
  window.dispatchEvent(new CustomEvent("mqx:theme", { detail: theme }));
}

export function resolvedDark(theme = document.documentElement.dataset.theme): boolean {
  if (theme === "light") {
    return false;
  }
  if (theme === "dark" || !theme) {
    return true;
  }
  return !window.matchMedia("(prefers-color-scheme: light)").matches;
}
