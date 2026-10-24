import { useState } from "react";
import { Trash2 } from "lucide-react";
import type { AppSettings, FlowModel, WritingModelDownloadId } from "../../types";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";
import { api } from "../../services/tauriApi";
import { llmSelectionPatch, selectedLlm } from "../../llmSelection";
import { CatalogDetails, MODEL_CATALOG } from "./modelCatalog";
import { writingSetupLabel } from "../SetupProgress";
import { Row } from "./ui";

export function WritingCatalog({
  task = "all",
  settings,
  intelligence,
  onUpdateSettings,
  onInstallRuntime,
}: {
  task?: "all" | "polish" | "notes";
  settings: AppSettings;
  intelligence: IntelligenceHub;
  onUpdateSettings: (settings: Partial<AppSettings>) => Promise<boolean>;
  onInstallRuntime: () => void;
}) {
  const [pending, setPending] = useState<string | null>(null);
  const [operation, setOperation] = useState<"download" | "select" | "delete" | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const run = async (
    id: string,
    kind: "download" | "select" | "delete",
    action: () => Promise<unknown>,
  ) => {
    setPending(id);
    setOperation(kind);
    try {
      await action();
      await intelligence.refresh();
    } catch (error) {
      intelligence.notifyToast("error", String(error));
    } finally {
      setPending(null);
      setOperation(null);
      setConfirm(null);
    }
  };
  return (
    <>
      <ul aria-label="Additional writing models" className="divide-y divide-line-soft">
        {MODEL_CATALOG.filter(
          (model) =>
            model.task === "writing" &&
            (task !== "notes" || model.id !== "minicpm5-2b") &&
            (task !== "polish" || model.id !== "gemma4-e4b") &&
            ["qwen3.5-4b", "minicpm5-2b", "gemma4-e4b"].includes(model.id),
        ).map((model) => {
          const id = model.id as WritingModelDownloadId;
          const state = intelligence.intelligenceTiers?.find((item) => item.model_id === id);
          const weights = state?.weights_installed ?? state?.installed ?? false;
          const ready = weights && Boolean(intelligence.flowStatus?.runtime_installed);
          const progress = intelligence.intelligenceDownloads?.[id];
          const busy =
            intelligence.activeDownloadTiers.has(id) ||
            (pending === id && operation === "download");
          const active =
            (task === "notes"
              ? settings.summary_model === "follow_writing"
                ? selectedLlm(settings)
                : settings.summary_model
              : selectedLlm(settings)) === id;
          return (
            <li key={id} className="py-4 first:pt-2 last:pb-0">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="min-w-0 flex-1 basis-48">
                  <p className="text-sm font-medium text-ink">{model.name}</p>
                  <p className="text-xs text-muted mt-1 leading-relaxed">{model.purpose}</p>
                  <p className="text-2xs text-muted mt-1">
                    {model.size} ·{" "}
                    {ready
                      ? "Installed"
                      : weights
                        ? "Weights only · needs runtime"
                        : !intelligence.flowStatus?.runtime_installed
                          ? "Needs writing runtime, then model"
                          : "Needs download"}
                  </p>
                </div>
                <div className="flex items-center gap-1.5">
                  {
                    <button
                      type="button"
                      className="btn btn-secondary !text-xs"
                      aria-label={
                        task === "notes" ? `Use ${model.name} for notes` : `Use ${model.name}`
                      }
                      aria-pressed={active}
                      disabled={pending !== null}
                      onClick={() =>
                        run(id, "select", () =>
                          onUpdateSettings(
                            task === "notes"
                              ? { summary_model: model.id as AppSettings["summary_model"] }
                              : llmSelectionPatch(settings, model.id as FlowModel),
                          ),
                        )
                      }
                    >
                      {active ? "Selected" : task === "notes" ? "Use for notes" : "Use for polish"}
                    </button>
                  }
                  {!ready &&
                    (weights ? (
                      <button
                        type="button"
                        className="btn btn-primary !text-xs"
                        disabled={intelligence.runtimeDownloadActive || settings.offline_mode}
                        onClick={onInstallRuntime}
                      >
                        Install runtime
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="btn btn-primary !text-xs"
                        aria-label={`Download ${model.name}`}
                        disabled={pending !== null || busy || settings.offline_mode}
                        onClick={() => run(id, "download", () => api.installIntelligenceModel(id))}
                      >
                        {busy
                          ? writingSetupLabel(progress?.phase ?? "starting")
                          : intelligence.flowStatus?.runtime_installed
                            ? "Download"
                            : "Set up & download"}
                      </button>
                    ))}
                  {busy && progress?.attempt_id && (
                    <button
                      type="button"
                      className="btn btn-secondary !text-xs"
                      onClick={() =>
                        api
                          .cancelIntelligenceDownload(id, progress.attempt_id!)
                          .catch((error) => intelligence.notifyToast("error", String(error)))
                      }
                    >
                      Cancel download
                    </button>
                  )}
                  {weights && (
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`Delete ${model.name}`}
                      disabled={pending !== null}
                      onClick={() => setConfirm(id)}
                    >
                      <Trash2 size={14} aria-hidden />
                    </button>
                  )}
                </div>
              </div>
              {busy && (
                <p className="text-xs text-accent mt-2" role="status">
                  {progress
                    ? `${writingSetupLabel(progress.phase)}${["checking_runtime", "installing_runtime"].includes(progress.phase) ? " · model download follows verification" : ` · ${progress.progress_pct}% · ${progress.speed_mbps.toFixed(1)} MB/s`}`
                    : "Preparing download…"}
                </p>
              )}
              {progress?.phase === "error" && (
                <p className="text-xs text-danger mt-2" role="alert">
                  {progress.error ?? "Download failed. Try again."}
                </p>
              )}
              <CatalogDetails model={model} />
              {confirm === id && (
                <div className="flex flex-wrap gap-2 items-center mt-3 text-xs">
                  <p className="text-danger">
                    Delete {model.name}? You can download it again later.
                  </p>
                  <button
                    type="button"
                    className="btn btn-danger !text-xs"
                    disabled={pending !== null}
                    onClick={() => run(id, "delete", () => api.removeIntelligenceModel(id))}
                  >
                    Delete
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary !text-xs"
                    onClick={() => setConfirm(null)}
                  >
                    Cancel
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      <Row
        label="Meeting notes model"
        hint="Used for history summaries and the default for meeting notes. It does not change text polish. Download its model above before using it."
      >
        <select
          className="field"
          value={settings.summary_model ?? "follow_writing"}
          onChange={(event) =>
            onUpdateSettings({ summary_model: event.target.value as AppSettings["summary_model"] })
          }
        >
          <option value="follow_writing">Follow text polish model</option>
          {MODEL_CATALOG.filter((model) => model.task === "writing" && !model.unavailable).map(
            (model) => (
              <option key={model.id} value={model.id}>
                {model.name}
              </option>
            ),
          )}
        </select>
      </Row>
    </>
  );
}
