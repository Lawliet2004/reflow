// React resolves its development vs production build via `NODE_ENV` at
// module load. A shell that exports `NODE_ENV=production` (common on this
// machine) silently swaps in the production bundle, which has no `React.act`
// and breaks every `@testing-library/react` render. Force the test env here —
// this file loads before any test module — so `npm run test` passes
// regardless of the ambient shell.
process.env.NODE_ENV = "test";

import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

// jsdom implements neither of these, and both are read during render by the
// reduce-motion and theme paths.
if (!window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

if (!window.requestAnimationFrame) {
  window.requestAnimationFrame = (cb: FrameRequestCallback) =>
    setTimeout(() => cb(performance.now()), 16) as unknown as number;
  window.cancelAnimationFrame = (handle: number) => clearTimeout(handle);
}
