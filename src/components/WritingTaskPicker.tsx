import { useId, useState } from "react";
import { AppSettings } from "../types";
import { LLM_NAMES } from "../llmSelection";
import {
  SPEECH_MODEL_GUIDANCE,
  selectedWritingTask,
  WRITING_TASKS,
  WritingTask,
  writingTaskPatch,
  WritingPreferences,
  hasCustomWritingInputs,
} from "../writingTasks";

interface Props {
  settings: WritingPreferences;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean> | void;
  disabled?: boolean;
}

export function WritingTaskPicker({ settings, onUpdateSettings, disabled }: Props) {
  const id = useId();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selected = selectedWritingTask(settings);
  const task = WRITING_TASKS.find((task) => task.id === selected);
  const change = async (value: WritingTask) => {
    setSaving(true);
    setError(null);
    try {
      if ((await onUpdateSettings(writingTaskPatch(settings, value))) === false) {
        setError("Could not save this writing task. Try again.");
      }
    } catch {
      setError("Could not save this writing task. Try again.");
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="writing-task" aria-busy={saving}>
      <div className="writing-task-control">
        <label htmlFor={id} className="text-sm font-medium text-ink">
          Writing task
        </label>
        <select
          id={id}
          className="field"
          value={selected}
          disabled={disabled || saving}
          aria-describedby={`${id}-description`}
          onChange={(event) => {
            void change(event.target.value as WritingTask);
          }}
        >
          {selected === "custom" && <option value="custom">Custom preferences</option>}
          {WRITING_TASKS.map((task) => (
            <option key={task.id} value={task.id}>
              {task.label}
            </option>
          ))}
        </select>
      </div>
      <p id={`${id}-description`} className="text-sm text-muted leading-relaxed" aria-live="polite">
        {task?.description ??
          "Your existing cleanup, tone or mode has been customized. Choose a task to apply its suggested settings."}
      </p>
      {task && (
        <p className="text-xs text-muted">
          Suggested cleanup: {LLM_NAMES[task.model]}. You can change the model independently.
        </p>
      )}
      {settings.default_mode_id && settings.default_mode_id !== "dictation" && (
        <p className="text-xs text-warning">
          The{" "}
          {settings.modes?.find((mode) => mode.id === settings.default_mode_id)?.name ??
            settings.default_mode_id}{" "}
          mode currently overrides this default. Choosing a task switches the default mode to
          Dictation.
        </p>
      )}
      {hasCustomWritingInputs(settings) && (
        <p className="text-xs text-warning">
          Choosing a task clears custom instructions, translation and captured context for this
          dictation. Other modes keep their preferences.
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <details className="writing-task-guide">
        <summary className="text-sm text-ink cursor-pointer">Examples &amp; model guide</summary>
        {task && (
          <div className="writing-task-example">
            <p className="text-xs text-muted">Illustrative example · actual output may vary</p>
            <p className="text-sm text-muted leading-relaxed">You say: {task.exampleIn}</p>
            <p className="text-sm text-ink leading-relaxed whitespace-pre-wrap">
              Result: {task.exampleOut}
            </p>
          </div>
        )}
        <p className="text-sm text-ink-2 leading-relaxed">
          <strong>Speech model (ASR)</strong> hears your words. {SPEECH_MODEL_GUIDANCE}
        </p>
        <p className="text-sm text-ink-2 leading-relaxed">
          <strong>Cleanup model (LLM)</strong> edits recognized text. Start with Qwen3.5 0.8B for
          natural cleanup; try 2B for developer prompts and longer writing if memory and latency
          permit. No LLM gives you basic cleanup with no rewrite wait.
        </p>
        <p className="text-xs text-muted leading-relaxed">
          Tasks change writing preferences, not your speech model or downloads. Explicit corrections
          can be repaired; an unclear intended word may remain. Literal code uses Coding mode. App
          and shortcut modes can have their own task preferences.
        </p>
      </details>
    </div>
  );
}
