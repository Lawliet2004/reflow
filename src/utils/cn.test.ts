import { describe, expect, it } from "vitest";
import { cn } from "./cn";

describe("cn utility", () => {
  it("merges static classes", () => {
    expect(cn("px-2 py-1", "bg-blue-500")).toBe("px-2 py-1 bg-blue-500");
  });

  it("handles conditional classes", () => {
    const isTrue = Boolean(1);
    const isFalse = Boolean(0);
    expect(cn("base", isTrue && "is-true", isFalse && "is-false")).toBe("base is-true");
  });

  it("merges tailwind conflicts correctly", () => {
    expect(cn("p-4", "p-2")).toBe("p-2");
    expect(cn("text-red-500", "text-blue-500")).toBe("text-blue-500");
  });
});
