import { useCallback, useEffect, useRef, useState } from "react";
import { DesktopIntegrationStatus } from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";

export function useDesktopIntegration() {
  const [status, setStatus] = useState<DesktopIntegrationStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const acceptStatus = useCallback((next: DesktopIntegrationStatus) => {
    revision.current++;
    setStatus(next);
    setError(null);
  }, []);
  useEffect(() => {
    let alive = true;
    const scope = createEventScope();
    void (async () => {
      await scope.listen<DesktopIntegrationStatus>("desktop:integration-changed", (next) => {
        acceptStatus(next);
      });
      if (!alive) return;
      const started = revision.current;
      try {
        const next = await api.getDesktopIntegrationStatus();
        if (alive && revision.current === started) setStatus(next);
      } catch {
        if (alive && revision.current === started)
          setError("Could not check desktop integration. Reopen Settings to try again.");
      }
    })();
    return () => {
      alive = false;
      scope.dispose();
    };
  }, [acceptStatus]);
  return { status, error, acceptStatus };
}
