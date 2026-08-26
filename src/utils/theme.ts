import { AccentColor, AppSettings, AppTheme, UIFontScale } from "../types";

let systemDarkMedia: MediaQueryList | null = null;
let currentListener: ((e: MediaQueryListEvent) => void) | null = null;

export function getResolvedTheme(theme: AppTheme): "light" | "dark" {
  if (theme === "dark") return "dark";
  if (theme === "light") return "light";
  if (typeof window !== "undefined" && window.matchMedia) {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return "light";
}

export function applyTheme(
  settings: Pick<
    AppSettings,
    "app_theme" | "accent_color" | "reduce_motion" | "ui_font_scale" | "overlay_theme"
  >
): () => void {
  if (typeof document === "undefined") return () => {};

  const root = document.documentElement;
  const resolved = getResolvedTheme(settings.app_theme || "system");

  // Apply dark / light class
  root.classList.toggle("dark", resolved === "dark");

  // Apply accent color
  const accent: AccentColor = settings.accent_color || "sky";
  root.dataset.accent = accent;

  // Apply font scale
  const fontScale: UIFontScale = settings.ui_font_scale || "normal";
  root.dataset.fontScale = fontScale;

  // Apply reduce motion
  root.classList.toggle("reduce-motion", Boolean(settings.reduce_motion));

  // Cache to localStorage for instant hydration on window load
  try {
    localStorage.setItem("reflow_app_theme", settings.app_theme || "system");
    localStorage.setItem("reflow_accent", accent);
    localStorage.setItem("reflow_font_scale", fontScale);
  } catch {
    /* ignore storage errors */
  }

  // Set overlay theme attribute if present
  if (settings.overlay_theme) {
    root.dataset.overlayTheme = settings.overlay_theme;
  }

  // Bind system media query listener if theme is system
  if (currentListener && systemDarkMedia) {
    systemDarkMedia.removeEventListener("change", currentListener);
    currentListener = null;
  }

  if (settings.app_theme === "system" && typeof window !== "undefined" && window.matchMedia) {
    systemDarkMedia = window.matchMedia("(prefers-color-scheme: dark)");
    currentListener = (e: MediaQueryListEvent) => {
      root.classList.toggle("dark", e.matches);
    };
    systemDarkMedia.addEventListener("change", currentListener);
  }

  return () => {
    if (currentListener && systemDarkMedia) {
      systemDarkMedia.removeEventListener("change", currentListener);
      currentListener = null;
    }
  };
}
