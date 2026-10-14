import { useEffect, useRef } from "react";
import { AppSettings, UI_ZOOM_DEFAULT, UI_ZOOM_STEP, clampUiZoom } from "../types";

/** Pixels of wheel delta that make one zoom step (~half a mouse notch). */
const WHEEL_STEP_PX = 50;
/** A pause this long means a new gesture: drop the accumulated remainder. */
const WHEEL_REST_MS = 250;

/**
 * Browser-style zoom for the main window. Ctrl+= / Ctrl+- step `ui_zoom`,
 * Ctrl+0 resets it, and holding Ctrl while scrolling follows the wheel — a
 * trackpad pinch arrives as a Ctrl+wheel too, so it works for free.
 *
 * The wheel listener is non-passive so the webview's own pinch-zoom never
 * fights ours. Listeners live for the window's lifetime; the latest zoom and
 * updater reach them through refs, synced in effects rather than during
 * render.
 */
export function useUiZoom(
  zoom: number | undefined,
  update: (s: Partial<AppSettings>) => void,
): void {
  const zoomRef = useRef(UI_ZOOM_DEFAULT);
  const updateRef = useRef(update);

  useEffect(() => {
    zoomRef.current = clampUiZoom(zoom ?? UI_ZOOM_DEFAULT);
  }, [zoom]);
  useEffect(() => {
    updateRef.current = update;
  }, [update]);

  useEffect(() => {
    const apply = (next: number) => {
      const clamped = clampUiZoom(next);
      if (clamped !== zoomRef.current) updateRef.current({ ui_zoom: clamped });
    };
    const onKeyDown = (e: KeyboardEvent) => {
      // Alt-free Ctrl/Cmd combos only; a prevented event is already claimed
      // (e.g. the hotkey recorder captures it first).
      if (e.defaultPrevented || e.altKey || !(e.ctrlKey || e.metaKey)) return;
      switch (e.code) {
        case "Equal": // Ctrl+= and Ctrl++ (Shift+=) both land here
        case "NumpadAdd":
          e.preventDefault();
          apply(zoomRef.current + UI_ZOOM_STEP);
          break;
        case "Minus":
        case "NumpadSubtract":
          e.preventDefault();
          apply(zoomRef.current - UI_ZOOM_STEP);
          break;
        case "Digit0":
        case "Numpad0":
          e.preventDefault();
          apply(UI_ZOOM_DEFAULT);
          break;
      }
    };
    let rest = 0;
    let lastTick = 0;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.defaultPrevented) return;
      e.preventDefault();
      const now = performance.now();
      if (now - lastTick > WHEEL_REST_MS) rest = 0;
      lastTick = now;
      // deltaMode 1 reports lines; one line is roughly a third of a notch.
      rest += e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY;
      if (Math.abs(rest) < WHEEL_STEP_PX) return;
      apply(zoomRef.current + (rest < 0 ? UI_ZOOM_STEP : -UI_ZOOM_STEP));
      rest = 0;
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("wheel", onWheel);
    };
  }, []);
}
