import { describe, expect, it } from "vitest";
import { applyTheme, getResolvedTheme } from "./theme";

describe("Theme system", () => {
  it("resolves explicit light and dark themes", () => {
    expect(getResolvedTheme("light")).toBe("light");
    expect(getResolvedTheme("dark")).toBe("dark");
  });

  it("applies font-scale, accent, and theme classes to root element", () => {
    const cleanup = applyTheme({
      app_theme: "dark",
      accent_color: "emerald",
      reduce_motion: true,
      ui_font_scale: "roomy",
      overlay_theme: "auto",
    });

    const root = document.documentElement;
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.dataset.accent).toBe("emerald");
    expect(root.dataset.fontScale).toBe("roomy");
    expect(root.classList.contains("reduce-motion")).toBe(true);

    cleanup();
  });
});
