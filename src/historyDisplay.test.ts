import { expect, it } from "vitest";
import { relativeTime } from "./historyDisplay";

it("formats recent times relatively and older ones as a date", () => {
  const now = Date.parse("2026-10-02T12:00:00Z");
  const ago = (ms: number) => new Date(now - ms).toISOString();
  expect(relativeTime(ago(20_000), now)).toBe("just now");
  expect(relativeTime(ago(5 * 60_000), now)).toMatch(/5/);
  expect(relativeTime(ago(3 * 3_600_000), now)).toMatch(/3/);
  expect(relativeTime(ago(30 * 86_400_000), now)).not.toMatch(/ago/);
});
