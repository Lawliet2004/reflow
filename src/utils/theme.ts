import { getCurrentWindow } from "@tauri-apps/api/window";
import { AccentColor, AppSettings, AppTheme, UIFontScale } from "../types";

/** Windows 11 reports platformVersion >= 13 through UA client hints (WebView2). */
async function isWindows11(): Promise<boolean> {
  const ua = (
    navigator as Navigator & {
      userAgentData?: {
        platform: string;
        getHighEntropyValues(hints: string[]): Promise<{ platformVersion?: string }>;
      };
    }
  ).userAgentData;
  if (ua?.platform !== "Windows") return false;
  const { platformVersion } = await ua.getHighEntropyValues(["platformVersion"]);
  return parseInt(platformVersion ?? "0", 10) >= 13;
}

/**
 * Desktop-only window chrome: Mica is painted by the OS behind a transparent
 * window (see `windowEffects` in tauri.conf.json) and follows the *window*
 * theme, so pin that to the app theme or dark text would land on light Mica.
 * `.mica` is only added on Windows 11; elsewhere the body stays opaque.
 */
export async function syncWindowChrome(theme: AppTheme): Promise<void> {
  try {
    await getCurrentWindow().setTheme(theme === "system" ? null : theme);
    document.documentElement.classList.toggle("mica", await isWindows11());
  } catch {
    /* Cosmetic only; the opaque background remains. */
  }
}

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
  >,
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
