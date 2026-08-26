import React from "react";
import { Mic, History, Settings, FolderOpen, Power, Cpu } from "lucide-react";
import { AppState, ModelStatus, isModelReady } from "../types";
import { api } from "../services/tauriApi";

export type NavTab = "dictate" | "history" | "settings";

interface NavigationProps {
  activeTab: NavTab;
  setActiveTab: (tab: NavTab) => void;
  appState: AppState;
  modelStatus: ModelStatus | null;
}

const ITEMS: { id: NavTab; label: string; icon: React.ReactNode }[] = [
  { id: "dictate", label: "Home", icon: <Mic className="w-4 h-4" /> },
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
  return { label: "Loading", tone: "text-amber-600 dark:text-amber-400" };
}

export const Navigation: React.FC<NavigationProps> = ({
  activeTab,
  setActiveTab,
  appState,
  modelStatus,
}) => {
  const badge = modelBadgeLabel(modelStatus);
  const live = appState === "RECORDING" || appState === "PROCESSING";

  return (
    <aside className="w-[148px] shrink-0 h-full flex flex-col border-r border-line bg-surface/60 backdrop-blur-md py-4 select-none">
      <nav className="flex flex-col gap-1 w-full px-2.5">
        {ITEMS.map((item) => {
          const active = activeTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setActiveTab(item.id)}
              aria-label={item.label}
              aria-current={active ? "page" : undefined}
              className={`relative w-full h-10 rounded-xl flex items-center gap-2.5 px-3 text-[13px] font-medium transition-all cursor-pointer ${
                active
                  ? "bg-accent-soft text-accent border border-accent-border font-semibold shadow-xs"
                  : "text-muted hover:text-ink hover:bg-base-2 border border-transparent"
              }`}
            >
              {active && (
                <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-5 rounded-r-full bg-accent shadow-[0_0_8px_var(--color-accent)]" />
              )}
              {item.icon}
              <span>{item.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="mt-auto px-2.5 pt-4 border-t border-line space-y-1.5">
        {badge && (
          <div
            className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-[11.5px] font-semibold"
            title="Speech model status"
          >
            <Cpu className="w-3.5 h-3.5 text-muted" />
            <span className={badge.tone}>{badge.label}</span>
          </div>
        )}
        <button
          onClick={() => api.openLogsFolder().catch(() => {})}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-[12px] text-muted hover:text-ink hover:bg-base-2 transition-colors cursor-pointer"
        >
          <FolderOpen className="w-3.5 h-3.5" />
          <span>Open logs</span>
        </button>
        <button
          onClick={() => api.quit().catch(() => {})}
          disabled={live}
          title={live ? "Cannot quit while recording" : "Quit Reflow"}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-[12px] text-rose-600 dark:text-rose-400 hover:bg-rose-500/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <Power className="w-3.5 h-3.5" />
          <span>Quit Reflow</span>
        </button>
      </div>
    </aside>
  );
};
