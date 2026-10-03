import React, { useState } from "react";
import { AppSettings, CustomReplacement } from "../../types";
import { Section, Toggle } from "./ui";
import { Plus, X, Trash2, Sliders, BookOpen } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => Promise<boolean>;
}

export const DictionaryPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const terms = settings.dictionary_terms;
  const replacements = settings.custom_replacements ?? [];
  const [newTerm, setNewTerm] = useState("");
  const [newBefore, setNewBefore] = useState("");
  const [newAfter, setNewAfter] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async (patch: Partial<AppSettings>, done?: () => void) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (await onUpdateSettings(patch)) done?.();
      else setError("Could not save your dictionary. Please try again.");
    } catch {
      setError("Could not save your dictionary. Please try again.");
    } finally {
      setBusy(false);
    }
  };
  const addTerm = () => {
    const term = newTerm.trim();
    if (!term || terms.some((item) => item.term.toLowerCase() === term.toLowerCase())) return;
    return save(
      {
        dictionary_terms: [
          ...terms,
          { id: crypto.randomUUID(), term, preferred_spelling: term, category: "Custom" },
        ],
      },
      () => setNewTerm(""),
    );
  };
  const removeTerm = (id: string) =>
    save({ dictionary_terms: terms.filter((term) => term.id !== id) });
  const addReplacement = () => {
    const before = newBefore.trim();
    const after = newAfter.trim();
    if (!before || !after) return;
    return save(
      {
        custom_replacements: [
          ...replacements,
          { id: crypto.randomUUID(), before, after, enabled: true },
        ],
      },
      () => {
        setNewBefore("");
        setNewAfter("");
      },
    );
  };
  const toggleReplacement = (rule: CustomReplacement) =>
    save({
      custom_replacements: replacements.map((item) =>
        item.id === rule.id ? { ...item, enabled: !item.enabled } : item,
      ),
    });
  const removeReplacement = (id: string) =>
    save({ custom_replacements: replacements.filter((item) => item.id !== id) });

  return (
    <fieldset disabled={busy} className="space-y-6 min-w-0">
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {!!settings.dictionary_suggestions?.length && (
        <Section
          title="Suggestions from corrections"
          description="Accept a spelling to teach the speech model your vocabulary."
        >
          {settings.dictionary_suggestions.map((item) => (
            <div
              key={`${item.before}:${item.after}`}
              className="flex items-center gap-2 py-2 text-sm"
            >
              <span className="flex-1">
                {item.before} → {item.after} <span className="text-muted">({item.frequency})</span>
              </span>
              <button
                className="btn btn-secondary"
                onClick={() =>
                  save({
                    dictionary_terms: [
                      ...terms,
                      {
                        id: crypto.randomUUID(),
                        term: item.before,
                        preferred_spelling: item.after,
                        category: "Correction",
                      },
                    ],
                    dictionary_suggestions: settings.dictionary_suggestions?.filter(
                      (s) => s !== item,
                    ),
                  })
                }
              >
                Accept
              </button>
              <button
                className="btn btn-ghost"
                onClick={() =>
                  save({
                    dictionary_suggestions: settings.dictionary_suggestions?.filter(
                      (s) => s !== item,
                    ),
                    dismissed_corrections: [
                      ...(settings.dismissed_corrections ?? []),
                      `${item.before}\n${item.after}`,
                    ],
                  })
                }
              >
                Dismiss
              </button>
            </div>
          ))}
        </Section>
      )}
      <Section
        icon={<BookOpen className="w-4 h-4" />}
        title="Vocabulary"
        description="Names and jargon passed as hotwords to the speech model."
      >
        <div className="flex flex-wrap gap-2">
          <input
            className="field flex-1 shadow-sm"
            placeholder="Add a name, word, or phrase"
            aria-label="New vocabulary term"
            value={newTerm}
            onChange={(e) => setNewTerm(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addTerm()}
          />
          <button
            className="btn btn-primary !px-3.5"
            onClick={addTerm}
            title="Add term"
            aria-label="Add term"
          >
            <Plus className="w-4 h-4" />
          </button>
        </div>
        {terms.length > 0 ? (
          <div className="flex flex-wrap gap-1.5 pt-1">
            {terms.map((t) => (
              <span
                key={t.id}
                className="chip !py-1.5 !bg-accent-soft !border-accent-border !text-accent"
              >
                {t.term}
                <button
                  onClick={() => removeTerm(t.id)}
                  className="text-accent hover:text-danger transition-colors cursor-pointer ml-1"
                  title="Remove term"
                  aria-label={`Remove term ${t.term}`}
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </span>
            ))}
          </div>
        ) : (
          <p className="text-xs text-muted italic">No custom terms added yet.</p>
        )}
      </Section>

      <Section
        icon={<Sliders className="w-4 h-4" />}
        title="Custom replacements"
        description='Replace spoken phrases after transcription (e.g. "git hub" → GitHub).'
      >
        <div className="flex flex-wrap gap-2">
          <input
            className="field flex-1 min-w-[120px]"
            placeholder="Before"
            aria-label="Spoken phrase"
            value={newBefore}
            onChange={(e) => setNewBefore(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addReplacement()}
          />
          <input
            className="field flex-1 min-w-[120px]"
            placeholder="After"
            aria-label="Replacement text"
            value={newAfter}
            onChange={(e) => setNewAfter(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addReplacement()}
          />
          <button
            className="btn btn-primary !px-3.5"
            onClick={addReplacement}
            title="Add replacement"
            aria-label="Add replacement"
          >
            <Plus className="w-4 h-4" />
          </button>
        </div>
        {replacements.length > 0 ? (
          <div className="divide-y divide-line rounded-xl border border-line overflow-hidden">
            {replacements.map((rule) => (
              <div key={rule.id} className="flex items-center gap-3 px-3 py-2.5">
                <p className="flex-1 min-w-0 text-sm text-ink-2 truncate">
                  <span className="text-muted">{rule.before}</span>
                  <span className="text-muted mx-1.5">→</span>
                  <span className="font-medium text-ink">{rule.after}</span>
                </p>
                <Toggle
                  on={rule.enabled}
                  onChange={() => toggleReplacement(rule)}
                  ariaLabel="Enable replacement"
                />
                <button
                  className="icon-btn hover:!bg-danger/10 hover:!text-danger"
                  onClick={() => removeReplacement(rule.id)}
                  title="Delete replacement"
                  aria-label="Delete replacement"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-xs text-muted italic">No replacements yet.</p>
        )}
      </Section>
    </fieldset>
  );
};
