import React, { useCallback, useEffect, useRef, useState } from "react";
import { AppSettings, ModelStatus } from "../types";
import type { IntelligenceHub } from "../hooks/useIntelligenceHub";
import { PAGES, PAGE_ICONS } from "./settings/ui";
import { ModesPage } from "./settings/ModesPage";
import { SnippetsPage } from "./settings/SnippetsPage";
import { OutputPage } from "./settings/OutputPage";
import { GeneralPage } from "./settings/GeneralPage";
import { AppearancePage } from "./settings/AppearancePage";
import { AudioPage } from "./settings/AudioPage";
import { ModelPage } from "./settings/ModelPage";
import { CleanupPage } from "./settings/CleanupPage";
import { DictionaryPage } from "./settings/DictionaryPage";
import { PhonePage } from "./settings/PhonePage";
import { ExpansionAdvanced } from "./settings/ExpansionAdvanced";
import { AdvancedPage } from "./settings/AdvancedPage";
import { Check, Search, X } from "lucide-react";

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
  onUpdateSettings: persistSettings,
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
  const searchInput = useRef<HTMLInputElement>(null);
  const [pendingSaves, setPendingSaves] = useState(0);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  const onUpdateSettings = useCallback(
    async (changes: Partial<AppSettings>) => {
      setPendingSaves((value) => value + 1);
      setSaveError(null);
      setJustSaved(false);
      try {
        const saved = await persistSettings(changes);
        if (!saved) setSaveError("Settings could not be saved. Check the values and try again.");
        setJustSaved(saved);
        return saved;
      } catch (failure) {
        setSaveError(`Settings could not be saved: ${String(failure)}`);
        return false;
      } finally {
        setPendingSaves((value) => value - 1);
      }
    },
    [persistSettings],
  );

  const q = query.trim().toLowerCase();
  const filtered = q
    ? PAGES.filter(
        (p) => p.label.toLowerCase().includes(q) || p.keywords.some((k) => k.includes(q)),
      )
    : PAGES;

  const page = filtered.some((item) => item.id === selectedPage) ? selectedPage : filtered[0]?.id;
  const current = PAGES.find((p) => p.id === page);

  useEffect(() => {
    if (!justSaved) return;
    const timer = setTimeout(() => setJustSaved(false), 1600);
    return () => clearTimeout(timer);
  }, [justSaved]);

  return (
    <div className="settings-layout">
      <nav aria-label="Settings categories" className="settings-navigation">
        <h1 className="text-lg font-semibold tracking-tight px-3 mb-4">Settings</h1>
        <div className="relative mb-2 px-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted pointer-events-none" />
          <input
            ref={searchInput}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search…"
            className="field w-full !pl-7 !pr-8 !py-1.5 !text-xs"
            aria-label="Search settings"
          />
          {query && (
            <button
              className="icon-btn absolute right-2 top-1/2 -translate-y-1/2 !w-6 !h-6"
              aria-label="Clear settings search"
              onClick={() => {
                setQuery("");
                searchInput.current?.focus();
              }}
            >
              <X size={12} aria-hidden />
            </button>
          )}
        </div>
        {filtered.map((item) => {
          const active = page === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setPage(item.id)}
              aria-current={active ? "page" : undefined}
              className={`sidebar-link sidebar-link-sm ${active ? "is-active" : ""}`}
            >
              {PAGE_ICONS[item.id]}
              {item.label}
            </button>
          );
        })}
        {filtered.length === 0 && <p className="text-xs text-muted px-2 py-1">No matches</p>}
      </nav>

      <div className="flex-1 min-w-0 overflow-y-auto select-text">
        <div className="settings-content space-y-6 animate-fade-rise">
          <header className="mb-1 flex flex-wrap items-end justify-between gap-4">
            <div className="min-w-0 flex-1 basis-48">
              <h2 className="font-display text-2xl font-semibold tracking-tight text-ink">
                {current?.label ?? "No results"}
              </h2>
              <p className="text-sm text-muted mt-2">
                {current?.description ?? "No settings match your search. Try a different word."}
              </p>
            </div>
            {/* Fixed slot, so saving never pushes the page down. */}
            <span className="save-status" role="status">
              {pendingSaves > 0 ? (
                "Saving…"
              ) : justSaved ? (
                <>
                  <Check className="w-3.5 h-3.5 text-success" aria-hidden /> Saved
                </>
              ) : null}
            </span>
          </header>

          {saveError && (
            <p role="alert" className="text-sm text-danger">
              {saveError}
            </p>
          )}
          <fieldset
            disabled={pendingSaves > 0}
            className="min-w-0 space-y-6"
            aria-busy={pendingSaves > 0}
            onClickCapture={(event) => {
              if (pendingSaves > 0) {
                event.preventDefault();
                event.stopPropagation();
              }
            }}
          >
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
            {page === "modes" && (
              <ModesPage settings={settings} onUpdateSettings={onUpdateSettings} />
            )}
            {page === "snippets" && (
              <SnippetsPage settings={settings} onUpdateSettings={onUpdateSettings} />
            )}
            {page === "output" && (
              <OutputPage settings={settings} onUpdateSettings={onUpdateSettings} />
            )}
            {page === "phone" && (
              <PhonePage settings={settings} onUpdateSettings={onUpdateSettings} />
            )}
            {page === "advanced" && (
              <>
                <AdvancedPage settings={settings} onUpdateSettings={onUpdateSettings} />
                <ExpansionAdvanced settings={settings} onUpdateSettings={onUpdateSettings} />
              </>
            )}
          </fieldset>
        </div>
      </div>
    </div>
  );
};
