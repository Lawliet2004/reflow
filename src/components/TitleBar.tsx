import React, { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "../services/tauriApi";
import logoUrl from "../assets/logo.svg";
import type { AppState } from "../types";

interface TitleBarProps {
  appState?: AppState;
}

const WindowControls: React.FC = () => {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const appWindow = getCurrentWindow();
        setIsMaximized(await appWindow.isMaximized());
        unlisten = await appWindow.onResized(async () => {
          try {
            setIsMaximized(await appWindow.isMaximized());
          } catch {
            /* ignore */
          }
        });
      } catch (err) {
        console.warn("Could not bind window resize listener:", err);
      }
    })();
    return () => unlisten?.();
  }, []);

  const minimize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!isTauri()) return;
    try {
      await getCurrentWindow().minimize();
    } catch (err) {
      console.error("Minimize failed:", err);
    }
  };

  const toggleMaximize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!isTauri()) return;
    try {
      const appWindow = getCurrentWindow();
      await appWindow.toggleMaximize();
      setIsMaximized(await appWindow.isMaximized());
    } catch (err) {
      console.error("Toggle maximize failed:", err);
    }
  };

  const close = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!isTauri()) return;
    try {
      await getCurrentWindow().close();
    } catch (err) {
      console.error("Close failed:", err);
    }
  };

  const btnClass =
    "h-full w-[46px] flex items-center justify-center text-muted hover:text-ink hover:bg-base-2 active:bg-surface-3 transition-colors cursor-pointer";

  return (
    <div className="flex items-center h-full" data-tauri-drag-region={false}>
      <button
        type="button"
        onClick={minimize}
        title="Minimize"
        aria-label="Minimize"
        className={btnClass}
      >
        <svg
          width="10"
          height="1"
          viewBox="0 0 10 1"
          className="shape-rendering-crispEdges"
          aria-hidden
        >
          <rect width="10" height="1" fill="currentColor" />
        </svg>
      </button>
      <button
        type="button"
        onClick={toggleMaximize}
        title={isMaximized ? "Restore" : "Maximize"}
        aria-label={isMaximized ? "Restore" : "Maximize"}
        className={btnClass}
      >
        {isMaximized ? (
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="1"
            aria-hidden
          >
            <rect x="2.5" y="0.5" width="7" height="7" rx="0.5" />
            <path d="M0.5 2.5v7h7" strokeLinecap="square" />
          </svg>
        ) : (
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="1"
            aria-hidden
          >
            <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
          </svg>
        )}
      </button>
      <button
        type="button"
        onClick={close}
        title="Close (hides to tray)"
        aria-label="Close"
        className={`${btnClass} hover:text-white hover:bg-[#e11d48] active:bg-[#be123c]`}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
          strokeLinecap="round"
          aria-hidden
        >
          <path d="M1 1l8 8M9 1L1 9" />
        </svg>
      </button>
    </div>
  );
};

export const TitleBar: React.FC<TitleBarProps> = () => {
  const handleDoubleClick = async () => {
    if (!isTauri()) return;
    try {
      const appWindow = getCurrentWindow();
      await appWindow.toggleMaximize();
    } catch (err) {
      console.error("Double click maximize failed:", err);
    }
  };

  return (
    <header
      data-tauri-drag-region
      onDoubleClick={handleDoubleClick}
      className="h-10 w-full shrink-0 flex items-center justify-between bg-surface/80 backdrop-blur-md border-b border-line select-none z-50 text-ink shadow-[0_1px_2px_rgba(0,0,0,0.02)]"
    >
      <div className="flex items-center gap-2.5 px-3.5 h-full" data-tauri-drag-region>
        <img
          src={logoUrl}
          alt="Reflow"
          className="w-4 h-4 rounded-[4px] pointer-events-none drop-shadow-sm"
          draggable={false}
        />
        <span data-tauri-drag-region className="text-[13px] font-semibold tracking-tight text-ink font-sans">
          Reflow
        </span>
      </div>

      <div className="flex-1 h-full" data-tauri-drag-region />

      <WindowControls />
    </header>
  );
};
