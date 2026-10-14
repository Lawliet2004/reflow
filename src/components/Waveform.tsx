import React, { useEffect, useLayoutEffect, useRef } from "react";

import { useViewVisibility } from "../hooks/useViewVisibility";

interface WaveformProps {
  level: number; // 0.0 – 1.0 RMS
  active: boolean;
  barCount?: number;
  height?: number;
  tone?: "light" | "dark";
  className?: string;
}

/**
 * Live voice waveform with sky/cyan equalizer bars.
 * Updates bar heights by direct DOM style writes inside one rAF loop —
 * no React re-renders per frame, zero idle CPU when rested.
 */
export const Waveform: React.FC<WaveformProps> = ({
  level,
  active,
  barCount = 24,
  height = 36,
  tone = "light",
}) => {
  const visible = useViewVisibility();
  const containerRef = useRef<HTMLDivElement>(null);
  const fullHeight = Math.max(1, height - 4);
  const levelRef = useRef(level);
  const activeRef = useRef(active);
  const rafRef = useRef(0);
  const tickFnRef = useRef<(() => void) | null>(null);

  useLayoutEffect(() => {
    levelRef.current = level;
    activeRef.current = active;
  }, [level, active]);

  useLayoutEffect(() => {
    if (!visible) return;
    const el = containerRef.current;
    if (!el) return;
    const bars = Array.from(el.children) as HTMLElement[];
    const heights = new Array(bars.length).fill(0.08);

    const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const reduced = () =>
      document.documentElement.classList.contains("reduce-motion") || motionPreference.matches;
    const tick = () => {
      const center = bars.length / 2;
      let moving = false;
      for (let i = 0; i < bars.length; i++) {
        const centerFactor = 1 - Math.abs(i - center) / center;
        const jitter = 0.55 + Math.random() * 0.9;
        const target = activeRef.current
          ? Math.min(1, levelRef.current * 3.4 * centerFactor * jitter + 0.08)
          : 0.08;
        const next = heights[i] + (target - heights[i]) * 0.16;
        if (Math.abs(next - heights[i]) > 0.001) moving = true;
        heights[i] = next;
        // Fixed bar geometry avoids re-laying out the scrolling surface on
        // each frame. The animation only changes its visual scale.
        const scale = Math.max(3, next * fullHeight) / fullHeight;
        bars[i].style.transform = `scaleY(${scale})`;
        let opacity: string;
        if (activeRef.current) {
          opacity = String(Math.max(0.4, next * 1.1));
        } else {
          opacity = tone === "dark" ? "0.45" : "0.3";
        }
        bars[i].style.opacity = opacity;
      }
      rafRef.current =
        !reduced() && (moving || activeRef.current) ? requestAnimationFrame(tick) : 0;
    };
    tickFnRef.current = tick;
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
      tickFnRef.current = null;
    };
  }, [barCount, fullHeight, tone, visible]);

  // Wake the rAF loop when level jumps up after a rest.
  useEffect(() => {
    if (visible && (active || level > 0.02) && !rafRef.current && tickFnRef.current) {
      rafRef.current = requestAnimationFrame(tickFnRef.current);
    }
  }, [active, level, visible]);

  const barColor = "bg-accent shadow-[0_0_8px_var(--color-accent)]";
  const idleBar = tone === "dark" ? "bg-accent/25" : "bg-line";
  const restScale = Math.max(3, 0.08 * fullHeight) / fullHeight;

  return (
    <div
      ref={containerRef}
      className="flex items-center justify-center gap-[3px] px-2"
      style={{ height: `${height}px` }}
      aria-hidden
    >
      {Array.from({ length: barCount }).map((_, idx) => (
        <div
          key={idx}
          className={`w-[3px] rounded-full transition-colors duration-75 ${
            active ? barColor : idleBar
          }`}
          style={{
            height: fullHeight,
            transform: `scaleY(${restScale})`,
            transformOrigin: "center",
          }}
        />
      ))}
    </div>
  );
};
