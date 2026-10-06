import { describe, expect, it } from "vitest";
import { APPEARANCE_DEFAULTS } from "../types";
import { applyTheme, contrastRatio, getResolvedTheme } from "./theme";

describe("Theme system", () => {
  it("resolves explicit light and dark themes", () => {
    expect(getResolvedTheme("light")).toBe("light");
    expect(getResolvedTheme("dark")).toBe("dark");
  });

  it("applies every appearance setting to the root element", () => {
    const cleanup = applyTheme({
      ...APPEARANCE_DEFAULTS,
      app_theme: "dark",
      accent_color: "custom",
      accent_custom: "#123456",
      reduce_motion: true,
      ui_font_scale: "roomy",
      surface_tone: "cool",
      reading_font: "mono",
      ui_density: "compact",
      corner_style: "round",
      hud_contrast: "high",
    });

    const root = document.documentElement;
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.dataset.accent).toBe("custom");
    expect(root.style.getPropertyValue("--accent-custom")).toBe("#123456");
    expect(root.dataset.fontScale).toBe("roomy");
    expect(root.dataset.tone).toBe("cool");
    expect(root.dataset.readingFont).toBe("mono");
    expect(root.dataset.density).toBe("compact");
    expect(root.dataset.corners).toBe("round");
    expect(root.dataset.hudContrast).toBe("high");
    expect(root.classList.contains("reduce-motion")).toBe(true);
    expect(JSON.parse(localStorage.getItem("reflow_appearance")!).data.tone).toBe("cool");

    cleanup();
  });

  it("computes WCAG contrast", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrastRatio("#777777", "#777777")).toBe(1);
  });
});
