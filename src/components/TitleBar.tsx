import React, { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "../services/tauriApi";
import { PanelLeft } from "lucide-react";

const WindowControls: React.FC = () => {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    (async () => {
      try {
        const appWindow = getCurrentWindow();
        const maximized = await appWindow.isMaximized();
        if (alive) setIsMaximized(maximized);
        unlisten = await appWindow.onResized(async () => {
          try {
            setIsMaximized(await appWindow.isMaximized());
          } catch {
            /* ignore */
          }
        });
        if (!alive) unlisten();
      } catch (err) {
        console.warn("Could not bind window resize listener:", err);
      }
    })();
    return () => {
      alive = false;
      unlisten?.();
    };
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

export const TitleBar: React.FC<{
  offline?: boolean;
  sidebarCollapsed?: boolean;
  onToggleSidebar?: () => void;
}> = ({ offline, sidebarCollapsed, onToggleSidebar }) => {
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
      className="titlebar h-10 w-full shrink-0 flex items-center justify-between select-none z-50 text-ink"
    >
      <div className="flex items-center gap-2.5 pl-3 pr-3.5 h-full" data-tauri-drag-region>
        {onToggleSidebar && (
          <button
            type="button"
            className="icon-btn"
            onClick={onToggleSidebar}
            aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-expanded={!sidebarCollapsed}
            aria-controls="app-sidebar"
          >
            <PanelLeft size={16} strokeWidth={1.75} aria-hidden />
          </button>
        )}
        {offline && (
          <span
            role="status"
            className="text-2xs text-accent"
            title="Downloads are blocked; local recognition and cleanup remain available"
          >
            Airplane mode
          </span>
        )}
        {!isTauri() && (
          <span className="text-2xs text-muted">Preview · dictation requires the desktop app</span>
        )}
      </div>

      <div className="flex-1 h-full" data-tauri-drag-region />

      <WindowControls />
    </header>
  );
};
