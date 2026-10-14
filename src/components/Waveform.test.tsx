import { act, render } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { Waveform } from "./Waveform";

it("responds to audio without changing bar layout on every animation frame", () => {
  const frames: FrameRequestCallback[] = [];
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.push(callback);
    return frames.length;
  });
  const view = render(<Waveform active level={0.5} barCount={4} height={22} />);
  const bars = Array.from(view.container.firstElementChild!.children) as HTMLElement[];
  const geometry = bars.map((bar) => bar.style.height);
  const first = bars.map((bar) => bar.style.transform);
  for (let frame = 0; frame < 4; frame++) act(() => frames.shift()?.(frame * 16));
  expect(bars.map((bar) => bar.style.height)).toEqual(geometry);
  expect(bars.map((bar) => bar.style.transform)).not.toEqual(first);
});

it("does not keep scheduling animation when reduced motion is enabled", () => {
  const frames: FrameRequestCallback[] = [];
  const request = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.push(callback);
    return frames.length;
  });
  document.documentElement.classList.add("reduce-motion");
  try {
    render(<Waveform active level={0.5} />);
    act(() => frames.shift()?.(0));
    expect(request).toHaveBeenCalledOnce();
  } finally {
    document.documentElement.classList.remove("reduce-motion");
  }
});
