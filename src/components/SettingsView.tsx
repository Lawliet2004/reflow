import React, { useEffect, useState } from "react";
import { AppSettings, IntelligenceTier, ModelStatus, RuntimeDownloadEvent } from "../types";
import type { IntelligenceDownloadEvent } from "../App";
import { PAGES, PAGE_ICONS } from "./settings/ui";
import { GeneralPage } from "./settings/GeneralPage";
import { AppearancePage } from "./settings/AppearancePage";
import { AudioPage } from "./settings/AudioPage";
import { ModelPage } from "./settings/ModelPage";
import { CleanupPage } from "./settings/CleanupPage";
import { DictionaryPage } from "./settings/DictionaryPage";
import { PhonePage } from "./settings/PhonePage";
import { AdvancedPage } from "./settings/AdvancedPage";
import { Search } from "lucide-react";

interface SettingsViewProps {
  settings: AppSettings;
  onUpdateSettings: (settings: Partial<AppSettings>) => void;
  modelStatus: ModelStatus | null;
  onReloadModel: () => void;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  activeDownloadTiers: Set<IntelligenceTier>;
  runtimeDownload: RuntimeDownloadEvent | null;
  runtimeDownloadActive: boolean;
  runtimeDownloadError: string | null;
  onInstallRuntime: () => void;
  onRemoveRuntime: () => void;
}

export const SettingsView: React.FC<SettingsViewProps> = ({
  settings,
  onUpdateSettings,
  modelStatus,
  onReloadModel,
  intelligenceDownload,
  activeDownloadTiers,
  runtimeDownload,
  runtimeDownloadActive,
  runtimeDownloadError,
  onInstallRuntime,
  onRemoveRuntime,
}) => {
  const [page, setPage] = useState<string>("general");
  const [query, setQuery] = useState("");

  const q = query.trim().toLowerCase();
  const filtered = q
    ? PAGES.filter(
        (p) =>
          p.label.toLowerCase().includes(q) ||
          p.keywords.some((k) => k.includes(q))
      )
    : PAGES;

  useEffect(() => {
    // If the search hides the active page, snap back to the first match.
    if (!filtered.some((p) => p.id === page)) {
      setPage(filtered[0]?.id ?? "general");
    }
  }, [query, filtered, page]);

  return (
    <div className="flex flex-1 min-h-0 w-full outline-none">
      <nav className="w-[160px] shrink-0 border-r border-line bg-surface/50 py-4 px-2 space-y-0.5 select-none">
        <div className="relative mb-2 px-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted pointer-events-none" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search…"
            className="field w-full !pl-7 !pr-2 !py-1.5 !text-[12px]"
            aria-label="Search settings"
          />
        </div>
        {filtered.map((item) => {
          const active = page === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setPage(item.id)}
              className={`w-full text-left px-3 py-2 rounded-lg text-[12.5px] font-medium cursor-pointer transition-colors flex items-center gap-2 ${
                active
                  ? "bg-accent-soft text-accent border border-accent-border font-semibold shadow-xs"
                  : "text-muted hover:text-ink hover:bg-base-2 border border-transparent"
              }`}
            >
              <span className={active ? "text-accent" : "text-muted"}>
                {PAGE_ICONS[item.id]}
              </span>
              {item.label}
            </button>
          );
        })}
        {filtered.length === 0 && (
          <p className="text-[11.5px] text-muted italic px-2 py-1">No matches</p>
        )}
      </nav>

      <div className="flex-1 min-w-0 overflow-y-auto select-text">
        <div className="max-w-2xl mx-auto px-7 py-8 space-y-6 animate-fade-rise">
          <header className="mb-1">
            <h1 className="font-display text-[22px] font-semibold tracking-tight text-ink">
              {PAGES.find((p) => p.id === page)?.label}
            </h1>
            <p className="text-[12.5px] text-muted mt-0.5">Preferences save automatically</p>
          </header>

          {page === "general" && <GeneralPage settings={settings} onUpdateSettings={onUpdateSettings} />}
          {page === "appearance" && (
            <AppearancePage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "audio" && <AudioPage settings={settings} onUpdateSettings={onUpdateSettings} />}
          {page === "model" && (
            <ModelPage
              settings={settings}
              onUpdateSettings={onUpdateSettings}
              modelStatus={modelStatus}
              onReloadModel={onReloadModel}
              intelligenceDownload={intelligenceDownload}
              activeDownloadTiers={activeDownloadTiers}
              runtimeDownload={runtimeDownload}
              runtimeDownloadActive={runtimeDownloadActive}
              runtimeDownloadError={runtimeDownloadError}
              onInstallRuntime={onInstallRuntime}
              onRemoveRuntime={onRemoveRuntime}
            />
          )}
          {page === "cleanup" && (
            <CleanupPage
              settings={settings}
              onUpdateSettings={onUpdateSettings}
              modelStatus={modelStatus}
              intelligenceDownload={intelligenceDownload}
              activeDownloadTiers={activeDownloadTiers}
              runtimeDownload={runtimeDownload}
              runtimeDownloadActive={runtimeDownloadActive}
              runtimeDownloadError={runtimeDownloadError}
              onInstallRuntime={onInstallRuntime}
              onRemoveRuntime={onRemoveRuntime}
            />
          )}
          {page === "dictionary" && (
            <DictionaryPage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "phone" && <PhonePage settings={settings} onUpdateSettings={onUpdateSettings} />}
          {page === "advanced" && (
            <AdvancedPage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
        </div>
      </div>
    </div>
  );
};
