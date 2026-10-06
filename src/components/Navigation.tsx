import React from "react";
import { Mic, History, Settings, FolderOpen, NotebookPen, Power } from "lucide-react";
import { AppState, ModelStatus, isModelReady } from "../types";
import { api } from "../services/tauriApi";
import logoUrl from "../assets/logo.png";

export type NavTab = "dictate" | "history" | "notes" | "settings";
export type PageTab = Exclude<NavTab, "settings">;

interface NavigationProps {
  activeTab: PageTab;
  setActiveTab: (tab: PageTab) => void;
  appState: AppState;
  modelStatus: ModelStatus | null;
  collapsed: boolean;
  settingsOpen: boolean;
  onOpenSettings: (page?: "general" | "model") => void;
}

const ICON = { size: 18, strokeWidth: 1.75, "aria-hidden": true } as const;

const ITEMS: { id: PageTab; label: string; icon: React.ReactNode }[] = [
  { id: "dictate", label: "Home", icon: <Mic {...ICON} /> },
  { id: "notes", label: "Notes", icon: <NotebookPen {...ICON} /> },
  { id: "history", label: "History", icon: <History {...ICON} /> },
];

function modelBadgeLabel(status: ModelStatus | null): string | null {
  if (!status?.installed || status.is_downloading) return null;
  if (!isModelReady(status)) return "Loading";
  const isGpu = /cuda|gpu/i.test(status.backend ?? "");
  return `${status.name?.includes("1.7") ? "1.7B" : "0.6B"} · ${isGpu ? "GPU" : "CPU"}`;
}

export const Navigation: React.FC<NavigationProps> = ({
  activeTab,
  setActiveTab,
  appState,
  modelStatus,
  collapsed,
  settingsOpen,
  onOpenSettings,
}) => {
  const badge = modelBadgeLabel(modelStatus);
  const live = appState === "RECORDING" || appState === "PROCESSING" || appState === "INJECTING";
  const pct = Math.round(modelStatus?.download_progress_pct ?? 0);

  return (
    <aside id="app-sidebar" className="app-sidebar" data-collapsed={collapsed}>
      <div className="sidebar-brand">
        <img src={logoUrl} alt="" aria-hidden draggable={false} className="pointer-events-none" />
        <span>Reflow</span>
      </div>
      <nav aria-label="Main navigation" className="flex flex-col gap-0.5">
        {ITEMS.map((item) => {
          const active = !settingsOpen && activeTab === item.id;
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

      <div className="sidebar-footer">
        {modelStatus?.is_downloading ? (
          <div className="sidebar-card" role="status">
            <strong>Downloading speech model</strong>
            <div className="sidebar-meter" aria-hidden>
              <div style={{ width: `${pct}%` }} />
            </div>
            <span className="tabular-nums text-muted">{pct}%</span>
          </div>
        ) : (
          modelStatus &&
          !modelStatus.installed &&
          activeTab !== "dictate" && (
            <div className="sidebar-card is-callout">
              <strong>Set up dictation</strong>
              <p>Download a speech model once. After that, everything runs on this computer.</p>
              <button className="btn btn-primary" onClick={() => onOpenSettings("model")}>
                Set up speech model
              </button>
            </div>
          )
        )}
        <hr />
        <button
          onClick={() => onOpenSettings()}
          aria-label="Settings"
          title={badge ? `Settings · speech model ${badge}` : "Settings"}
          aria-haspopup="dialog"
          className={`sidebar-link ${settingsOpen ? "is-active" : ""}`}
        >
          <Settings {...ICON} />
          <span>Settings</span>
          {badge && <span className="sidebar-link-meta">{badge}</span>}
        </button>
        <button
          onClick={() => api.openLogsFolder().catch(() => {})}
          aria-label="Open logs"
          title="Open logs"
          className="sidebar-link"
        >
          <FolderOpen {...ICON} />
          <span>Open logs</span>
        </button>
        <button
          onClick={() => api.quit().catch(() => {})}
          disabled={live}
          aria-label="Quit Reflow"
          title={live ? "Finish the current session before quitting" : "Quit Reflow"}
          className="sidebar-link sidebar-quit"
        >
          <Power {...ICON} />
          <span>Quit Reflow</span>
        </button>
      </div>
    </aside>
  );
};
