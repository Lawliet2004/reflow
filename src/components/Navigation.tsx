import React from "react";
import { Mic, History, Settings, FolderOpen, NotebookPen, Power, Cpu } from "lucide-react";
import { AppState, ModelStatus, isModelReady } from "../types";
import { api } from "../services/tauriApi";

export type NavTab = "dictate" | "history" | "notes" | "settings";

interface NavigationProps {
  activeTab: NavTab;
  setActiveTab: (tab: NavTab) => void;
  appState: AppState;
  modelStatus: ModelStatus | null;
}

const ITEMS: { id: NavTab; label: string; icon: React.ReactNode }[] = [
  { id: "dictate", label: "Home", icon: <Mic className="w-4 h-4" /> },
  { id: "notes", label: "Notes", icon: <NotebookPen className="w-4 h-4" /> },
  { id: "history", label: "History", icon: <History className="w-4 h-4" /> },
  { id: "settings", label: "Settings", icon: <Settings className="w-4 h-4" /> },
];

function modelBadgeLabel(status: ModelStatus | null): { label: string; tone: string } | null {
  if (!status) return null;
  if (status.is_downloading) {
    return { label: `Downloading ${status.download_progress_pct ?? 0}%`, tone: "text-accent" };
  }
  if (!status.installed) return { label: "No model", tone: "text-muted" };
  if (isModelReady(status)) {
    const backend = status.backend ?? "ready";
    const isGpu = /cuda|gpu/i.test(backend);
    return {
      label: `${status.name?.includes("1.7") ? "1.7B" : "0.6B"} · ${isGpu ? "GPU" : "CPU"}`,
      tone: isGpu ? "text-accent font-semibold" : "text-muted",
    };
  }
  return { label: "Loading", tone: "text-warning" };
}

export const Navigation: React.FC<NavigationProps> = ({
  activeTab,
  setActiveTab,
  appState,
  modelStatus,
}) => {
  const badge = modelBadgeLabel(modelStatus);
  const live = appState === "RECORDING" || appState === "PROCESSING" || appState === "INJECTING";

  return (
    <aside className="app-sidebar">
      <div className="sidebar-heading">WORKSPACE</div>
      <nav aria-label="Main navigation" className="flex flex-col gap-1 w-full px-3">
        {ITEMS.map((item) => {
          const active = activeTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setActiveTab(item.id)}
              aria-label={item.label}
              title={item.label}
              aria-current={active ? "page" : undefined}
              className={`sidebar-link ${active ? "is-active" : ""}`}
            >
              {item.icon}
              <span>{item.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="sidebar-footer mt-auto px-3 pt-4 space-y-1.5">
        {badge && (
          <div
            className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs font-semibold"
            title="Speech model status"
          >
            <Cpu className="w-3.5 h-3.5 text-muted" />
            <span className={badge.tone}>{badge.label}</span>
          </div>
        )}
        <button
          onClick={() => api.openLogsFolder().catch(() => {})}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs text-muted hover:text-ink hover:bg-base-2 transition-colors cursor-pointer"
        >
          <FolderOpen className="w-3.5 h-3.5" />
          <span>Open logs</span>
        </button>
        <button
          onClick={() => api.quit().catch(() => {})}
          disabled={live}
          title={live ? "Finish the current session before quitting" : "Quit Reflow"}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs text-danger hover:bg-danger/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <Power className="w-3.5 h-3.5" />
          <span>Quit Reflow</span>
        </button>
      </div>
    </aside>
  );
};
