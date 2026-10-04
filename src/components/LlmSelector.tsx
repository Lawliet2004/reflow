import { useId } from "react";
import { AppSettings, FlowModel, IntelligenceTierState } from "../types";
import { LLM_NAMES, llmSelectionPatch, selectedLlm } from "../llmSelection";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean> | void;
  tiers?: IntelligenceTierState[] | null;
  disabled?: boolean;
  onOpenSettings?: () => void;
}

export function LlmSelector({
  settings,
  onUpdateSettings,
  tiers,
  disabled,
  onOpenSettings,
}: Props) {
  const id = useId();
  const model = selectedLlm(settings);
  const state = tiers?.find((tier) => tier.model_id === model);
  const needsInstall =
    model !== "none" && tiers != null && !state?.installed && !state?.downloading;
  return (
    <div className="llm-selector">
      <label htmlFor={id} className="text-sm font-medium text-ink">
        LLM model
      </label>
      <select
        id={id}
        className="field w-full"
        aria-describedby={`${id}-help`}
        value={model}
        disabled={disabled}
        onChange={(event) =>
          onUpdateSettings(llmSelectionPatch(settings, event.target.value as FlowModel))
        }
      >
        {Object.entries(LLM_NAMES).map(([value, label]) => (
          <option key={value} value={value}>
            {label}
          </option>
        ))}
      </select>
      <p id={`${id}-help`} className="text-xs text-muted" aria-live="polite">
        {model === "none"
          ? "Speech recognition and basic cleanup work without an LLM."
          : state?.downloading
            ? "Downloading. Speech recognition works while you wait."
            : needsInstall
              ? "Not installed. Speech recognition works without AI rewriting."
              : state?.installed
                ? "Rewrites your transcript locally after speech recognition."
                : "Optional local rewriting. Checking whether this model is installed…"}
        {needsInstall &&
          (onOpenSettings ? (
            <>
              {" "}
              <button
                type="button"
                className="text-accent underline underline-offset-2"
                onClick={onOpenSettings}
              >
                Download in Performance
              </button>
            </>
          ) : (
            " Download it in Settings → Performance when you want to use it."
          ))}
      </p>
    </div>
  );
}
