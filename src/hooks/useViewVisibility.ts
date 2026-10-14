import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";

let nativeVisible = true;
let visible = true;
let stop: (() => void) | undefined;
const subscribers = new Set<() => void>();

function refresh() {
  const next = nativeVisible && document.visibilityState !== "hidden";
  document.documentElement.classList.toggle("view-hidden", !next);
  if (next === visible) return;
  visible = next;
  subscribers.forEach((subscriber) => subscriber());
}

function subscribe(subscriber: () => void) {
  subscribers.add(subscriber);
  if (subscribers.size === 1) {
    const scope = createEventScope();
    let alive = true;
    let revision = 0;
    nativeVisible = true;
    document.addEventListener("visibilitychange", refresh);
    refresh();
    if (isTauri()) {
      void (async () => {
        await scope.listen<boolean>("window:visibility", (next) => {
          revision++;
          nativeVisible = next;
          refresh();
        });
        if (!alive) return;
        const started = revision;
        try {
          const window = getCurrentWindow();
          const [shown, minimized] = await Promise.all([window.isVisible(), window.isMinimized()]);
          if (alive && revision === started) {
            nativeVisible = shown && !minimized;
            refresh();
          }
        } catch {
          /* Document visibility remains a fallback. */
        }
      })();
    }
    stop = () => {
      alive = false;
      scope.dispose();
      document.removeEventListener("visibilitychange", refresh);
    };
  }
  return () => {
    subscribers.delete(subscriber);
    if (subscribers.size === 0) {
      stop?.();
      stop = undefined;
    }
  };
}

export function useViewVisibility() {
  return useSyncExternalStore(
    subscribe,
    () => visible,
    () => true,
  );
}
