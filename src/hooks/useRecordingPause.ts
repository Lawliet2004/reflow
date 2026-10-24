import { useEffect, useState } from "react";
import { RecordingPauseState } from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";

/** Shared by both windows; live events win over a delayed startup snapshot. */
export function useRecordingPause(): RecordingPauseState {
  const [state, setState] = useState<RecordingPauseState>({ can_pause: false, paused: false });
  useEffect(() => {
    const scope = createEventScope();
    let active = true;
    let events = 0;
    const setup = async () => {
      await scope.listen<RecordingPauseState>("recording:pause-state", (value) => {
        events++;
        setState(value);
      });
      if (!active) return;
      const version = events;
      await api
        .getRecordingPauseState()
        .then((value) => {
          if (active && events === version) setState(value);
        })
        .catch(() => {});
    };
    void setup();
    return () => {
      active = false;
      scope.dispose();
    };
  }, []);
  return state;
}
