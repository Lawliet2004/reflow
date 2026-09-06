import React, { useState } from "react";
import { AppSettings, ModelStatus } from "../types";
import type { IntelligenceHub } from "../hooks/useIntelligenceHub";
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
  onUpdateSettings: (settings: Partial<AppSettings>) => Promise<boolean>;
  modelStatus: ModelStatus | null;
  onReloadModel: () => void;
  /** Single intelligence hub — replaces five drilled download props. */
  intelligence: IntelligenceHub;
  onInstallRuntime: () => void;
  onRemoveRuntime: () => void;
}

export const SettingsView: React.FC<SettingsViewProps> = ({
  settings,
  onUpdateSettings,
  modelStatus,
  onReloadModel,
  intelligence,
  onInstallRuntime,
  onRemoveRuntime,
}) => {
  const {
    intelligenceDownload,
    activeDownloadTiers,
    runtimeDownload,
    runtimeDownloadActive,
    runtimeDownloadError,
  } = intelligence;
  const [selectedPage, setPage] = useState<string>("general");
  const [query, setQuery] = useState("");

  const q = query.trim().toLowerCase();
  const filtered = q
    ? PAGES.filter(
        (p) => p.label.toLowerCase().includes(q) || p.keywords.some((k) => k.includes(q)),
      )
    : PAGES;

  const page = filtered.some((item) => item.id === selectedPage) ? selectedPage : filtered[0]?.id;

  return (
    <div className="settings-layout">
      <nav aria-label="Settings categories" className="settings-navigation">
        <h1 className="text-xl font-semibold tracking-tight px-3 mb-5">Settings</h1>
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
              aria-current={active ? "page" : undefined}
              className={`w-full text-left px-3 py-2 rounded-lg text-[12.5px] font-medium cursor-pointer transition-colors flex items-center gap-2 ${
                active
                  ? "bg-accent-soft text-accent border border-accent-border font-semibold shadow-xs"
                  : "text-muted hover:text-ink hover:bg-base-2 border border-transparent"
              }`}
            >
              <span className={active ? "text-accent" : "text-muted"}>{PAGE_ICONS[item.id]}</span>
              {item.label}
            </button>
          );
        })}
        {filtered.length === 0 && (
          <p className="text-[11.5px] text-muted italic px-2 py-1">No matches</p>
        )}
      </nav>

      <div className="flex-1 min-w-0 overflow-y-auto select-text">
        <div className="settings-content space-y-6 animate-fade-rise">
          <header className="mb-1">
            <h2 className="font-display text-[28px] font-semibold tracking-tight text-ink">
              {PAGES.find((p) => p.id === page)?.label ?? "No results"}
            </h2>
            <p className="text-sm text-muted mt-2">
              {page
                ? "Make Reflow feel like your own. Changes save automatically."
                : "No settings match your search. Try a different word."}
            </p>
          </header>

          {page === "general" && (
            <GeneralPage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "appearance" && (
            <AppearancePage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "audio" && (
            <AudioPage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "model" && (
            <ModelPage
              intelligence={intelligence}
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
              intelligence={intelligence}
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
          {page === "phone" && (
            <PhonePage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
          {page === "advanced" && (
            <AdvancedPage settings={settings} onUpdateSettings={onUpdateSettings} />
          )}
        </div>
      </div>
    </div>
  );
};
