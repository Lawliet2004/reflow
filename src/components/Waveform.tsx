import React, { useEffect, useLayoutEffect, useRef } from "react";

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
  const containerRef = useRef<HTMLDivElement>(null);
  const levelRef = useRef(level);
  const activeRef = useRef(active);
  const rafRef = useRef(0);
  const tickFnRef = useRef<(() => void) | null>(null);

  levelRef.current = level;
  activeRef.current = active;

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const bars = Array.from(el.children) as HTMLElement[];
    const heights = new Array(bars.length).fill(0.08);

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
        bars[i].style.height = `${Math.max(3, next * (height - 4))}px`;
        bars[i].style.opacity = activeRef.current
          ? String(Math.max(0.4, next * 1.1))
          : tone === "dark"
            ? "0.45"
            : "0.3";
      }
      rafRef.current = moving || activeRef.current ? requestAnimationFrame(tick) : 0;
    };
    tickFnRef.current = tick;
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
      tickFnRef.current = null;
    };
  }, [barCount, height, tone]);

  // Wake the rAF loop when level jumps up after a rest.
  useEffect(() => {
    if ((active || level > 0.02) && !rafRef.current && tickFnRef.current) {
      rafRef.current = requestAnimationFrame(tickFnRef.current);
    }
  }, [active, level]);

  const barColor = "bg-accent shadow-[0_0_8px_var(--color-accent)]";
  const idleBar = tone === "dark" ? "bg-accent/25" : "bg-line";

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
          className={`w-[3px] rounded-full transition-all duration-75 ${
            active ? barColor : idleBar
          }`}
          style={{ height: 3 }}
        />
      ))}
    </div>
  );
};
