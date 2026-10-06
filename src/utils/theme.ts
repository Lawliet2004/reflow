import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppTheme, Appearance, WindowMaterial } from "../types";

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
 * `.mica` is only added on Windows 11 and when the user keeps the Mica
 * material; otherwise the body stays opaque.
 */
export async function syncWindowChrome(
  theme: AppTheme,
  material: WindowMaterial = "mica",
): Promise<void> {
  try {
    await getCurrentWindow().setTheme(theme === "system" ? null : theme);
    document.documentElement.classList.toggle("mica", material === "mica" && (await isWindows11()));
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

/** WCAG contrast ratio between two `#rrggbb` colours. */
export function contrastRatio(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, b] = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The root attributes every window derives from the appearance settings. */
export function appearanceDataset(a: Appearance): Record<string, string> {
  return {
    accent: a.accent_color,
    tone: a.surface_tone,
    readingFont: a.reading_font,
    headingFont: a.heading_font,
    fontScale: a.ui_font_scale,
    density: a.ui_density,
    corners: a.corner_style,
    overlayTheme: a.overlay_theme,
    overlayPosition: a.overlay_position,
    hudContrast: a.hud_contrast,
  };
}

export function applyTheme(a: Appearance): () => void {
  if (typeof document === "undefined") return () => {};

  const root = document.documentElement;
  root.classList.toggle("dark", getResolvedTheme(a.app_theme) === "dark");
  root.classList.toggle("reduce-motion", a.reduce_motion);
  const data = appearanceDataset(a);
  Object.assign(root.dataset, data);
  root.style.setProperty("--accent-custom", a.accent_custom);

  // Cached for index.html, which applies it before first paint.
  try {
    localStorage.setItem(
      "reflow_appearance",
      JSON.stringify({ theme: a.app_theme, data, accentCustom: a.accent_custom }),
    );
  } catch {
    /* ignore storage errors */
  }

  if (currentListener && systemDarkMedia) {
    systemDarkMedia.removeEventListener("change", currentListener);
    currentListener = null;
  }

  if (a.app_theme === "system" && typeof window !== "undefined" && window.matchMedia) {
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
