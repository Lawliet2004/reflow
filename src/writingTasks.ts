import { AppSettings, DictationMode, FlowModel, Mode } from "./types";
import { llmSelectionPatch, selectedLlm } from "./llmSelection";

export type WritingTask = "quick" | "natural" | "developer" | "writing" | "original";
export interface WritingTaskInfo {
  id: WritingTask;
  label: string;
  description: string;
  exampleIn: string;
  exampleOut: string;
  model: FlowModel;
  mode: DictationMode;
  level: AppSettings["cleanup_level"];
}

export const WRITING_TASKS: WritingTaskInfo[] = [
  {
    id: "quick",
    label: "Quick dictation",
    description: "Messages and quick notes. Tidy fillers and punctuation with no AI rewrite wait.",
    exampleIn: "um can you check the PR",
    exampleOut: "Can you check the PR?",
    model: "none",
    mode: "normal",
    level: "light",
  },
  {
    id: "natural",
    label: "Natural cleanup",
    description:
      "Everyday dictation. Removes repeats, false starts and self-corrections and fixes grammar in a casual tone.",
    exampleIn: "I was thinking we could we could ship on Friday I mean Thursday",
    exampleOut: "I was thinking we could ship on Thursday.",
    model: "qwen3.5-0.8b",
    mode: "normal",
    level: "medium",
  },
  {
    id: "developer",
    label: "Developer prompt",
    description:
      "Instructions for an AI coding assistant. Clarify the wording, fixes misheard technical terms, and preserves every requirement.",
    exampleIn: "fix the login um don't change getUserById and keep the API the same",
    exampleOut: "Fix the login. Don't change getUserById, and keep the API the same.",
    model: "qwen3.5-2b",
    mode: "developer_prompt",
    level: "medium",
  },
  {
    id: "writing",
    label: "Polished writing",
    description:
      "Emails, articles and longer notes. Formal grammar, readable paragraphs, • bullet lists and email layout, without adding ideas.",
    exampleIn:
      "I wanted to share an update the release is ready next I need your feedback before we publish",
    exampleOut:
      "I wanted to share an update: the release is ready.\n\nI need your feedback before we publish.",
    model: "qwen3.5-2b",
    mode: "notes",
    level: "high",
  },
  {
    id: "original",
    label: "Original words",
    description:
      "Keep the ASR wording without filler removal, dictionary replacements or AI rewriting. Saved snippets still expand; recognition can still make mistakes.",
    exampleIn: "um I think we should wait",
    exampleOut: "um I think we should wait",
    model: "none",
    mode: "normal",
    level: "raw",
  },
];

export type WritingPreferences = Partial<
  Pick<
    AppSettings,
    | "preset"
    | "cleanup_level"
    | "intelligence_tier"
    | "dictation_mode"
    | "style"
    | "default_mode_id"
    | "modes"
    | "auto_style_from_app"
  >
> &
  Partial<Pick<Mode, "custom_instructions" | "translate_to" | "context">>;

export function hasCustomWritingInputs(settings: WritingPreferences): boolean {
  const mode = settings.modes?.find((mode) => mode.id === "dictation");
  return Boolean(
    settings.custom_instructions?.trim() ||
    settings.translate_to ||
    Object.values(settings.context ?? {}).some(Boolean) ||
    mode?.custom_instructions.trim() ||
    mode?.translate_to ||
    Object.values(mode?.context ?? {}).some(Boolean),
  );
}

// Derive the task from existing persisted settings. Changing model size does
// not change the task; custom modes and old preferences are never migrated away.
export function selectedWritingTask(settings: WritingPreferences): WritingTask | "custom" {
  if (
    hasCustomWritingInputs(settings) ||
    (settings.default_mode_id && settings.default_mode_id !== "dictation")
  )
    return "custom";
  const llm = selectedLlm(settings);
  const level = settings.cleanup_level;
  if (level === "raw") return "original";
  if (settings.auto_style_from_app) return "custom";
  if (settings.dictation_mode === "coding") return "custom";
  if (level === "light" && llm === "none") return "quick";
  if (llm === "none" || !["faithful", "neutral"].includes(settings.style ?? "neutral"))
    return "custom";
  if (settings.dictation_mode === "developer_prompt" && level === "medium") return "developer";
  if (settings.dictation_mode === "notes" && level === "high") return "writing";
  if (settings.dictation_mode === "normal" && level === "medium") return "natural";
  return "custom";
}

export function writingTaskPatch(
  settings: WritingPreferences,
  id: WritingTask,
): Partial<AppSettings> {
  const task = WRITING_TASKS.find((task) => task.id === id)!;
  return {
    ...llmSelectionPatch(settings, task.model),
    cleanup_level: task.level,
    dictation_mode: task.mode,
    style: "faithful",
    auto_style_from_app: false,
    default_mode_id: "dictation",
    ...(settings.modes
      ? {
          modes: settings.modes.map((mode) =>
            mode.id === "dictation"
              ? {
                  ...mode,
                  custom_instructions: "",
                  translate_to: null,
                  context: { selected_text: false, clipboard: false, window_title: false },
                }
              : mode,
          ),
        }
      : {}),
  };
}

export const SPEECH_MODEL_GUIDANCE =
  "Start with Qwen3-ASR 0.6B for everyday dictation. Try 1.7B when technical names or mixed-language speech are being misheard and your computer has enough memory. Zipformer 20M is a compact English option for CPU streaming; Phonon-2 is another English-only alternative. Recognition hotword hints are unavailable with these English models. Add recurring names in Dictionary.";
