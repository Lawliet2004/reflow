import { safeListen } from "./tauriApi";

/** Owns asynchronous Tauri registrations, including those resolving after unmount. */
export function createEventScope() {
  let closed = false;
  const listeners = new Set<() => void>();
  return {
    async listen<T>(event: string, handler: (payload: T) => void): Promise<void> {
      try {
        const unsubscribe = await safeListen<T>(event, (payload) => {
          if (!closed) handler(payload);
        });
        if (closed) unsubscribe();
        else listeners.add(unsubscribe);
      } catch (error) {
        console.error(`Could not subscribe to ${event}`, error);
      }
    },
    dispose() {
      closed = true;
      listeners.forEach((unsubscribe) => unsubscribe());
      listeners.clear();
    },
  };
}
