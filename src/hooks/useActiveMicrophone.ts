import { useEffect, useState } from "react";
import { api } from "../services/tauriApi";
import { useViewVisibility } from "./useViewVisibility";

/**
 * Resolves the microphone dictation uses. While recording this is the device
 * the stream actually opened; idle, it is what the current selection would
 * capture from. Refreshes when the selection changes, when recording starts
 * (the live device is published as the stream opens) and when the view becomes
 * visible again — a headset may have been plugged in while the window was away.
 */
export function useActiveMicrophone(deviceId: string | null, recording: boolean): string | null {
  const [name, setName] = useState<string | null>(null);
  const visible = useViewVisibility();
  useEffect(() => {
    if (!visible) return;
    let alive = true;
    const timer = window.setTimeout(
      () => {
        api
          .getActiveMicrophone()
          .then((value) => {
            if (alive) setName(value || null);
          })
          .catch(() => {
            if (alive) setName(null);
          });
      },
      recording ? 400 : 0,
    );
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [deviceId, recording, visible]);
  return name;
}
