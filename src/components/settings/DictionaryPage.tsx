import React, { useEffect, useState } from "react";
import { AppSettings, CustomReplacement, DictionaryTerm } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Toggle } from "./ui";
import { Plus, X, Trash2, Sliders } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const DictionaryPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [terms, setTerms] = useState<DictionaryTerm[]>(settings.dictionary_terms);
  const [replacements, setReplacements] = useState<CustomReplacement[]>(
    settings.custom_replacements ?? []
  );
  const [newTerm, setNewTerm] = useState("");
  const [newBefore, setNewBefore] = useState("");
  const [newAfter, setNewAfter] = useState("");
  const [replacementsEnabled, setReplacementsEnabled] = useState(true);

  useEffect(() => setTerms(settings.dictionary_terms), [settings.dictionary_terms]);
  useEffect(
    () => setReplacements(settings.custom_replacements ?? []),
    [settings.custom_replacements]
  );

  const addTerm = async () => {
    const t = newTerm.trim();
    if (!t) return;
    try {
      const saved = await api.saveDictionaryTerm({
        id: "",
        term: t,
        preferred_spelling: t,
        category: "Custom",
      });
      setTerms((prev) => [...prev, saved]);
    } catch (e) {
      console.error("Save term failed:", e);
    }
    setNewTerm("");
  };

  const removeTerm = (id: string) => {
    setTerms((prev) => prev.filter((t) => t.id !== id));
    api.deleteDictionaryTerm(id).catch(() => {});
  };

  const addReplacement = async () => {
    const before = newBefore.trim();
    const after = newAfter.trim();
    if (!before || !after) return;
    try {
      const saved = await api.saveCustomReplacement({
        id: "",
        before,
        after,
        enabled: replacementsEnabled,
      });
      setReplacements((prev) => [...prev, saved]);
      setNewBefore("");
      setNewAfter("");
    } catch (e) {
      console.error("Save replacement failed:", e);
    }
  };

  const toggleReplacement = async (rule: CustomReplacement) => {
    const next = { ...rule, enabled: !rule.enabled };
    setReplacements((prev) => prev.map((r) => (r.id === rule.id ? next : r)));
    try {
      await api.saveCustomReplacement(next);
    } catch (e) {
      console.error("Update replacement failed:", e);
    }
  };

  const removeReplacement = (id: string) => {
    setReplacements((prev) => prev.filter((r) => r.id !== id));
    api.deleteCustomReplacement(id).catch(() => {});
  };

  return (
    <>
      <Section
        icon={<span className="text-sky-600">📖</span>}
        title="Vocabulary"
        description="Names and jargon passed as hotwords to the speech model."
      >
        <div className="flex gap-2">
          <input
            className="field flex-1 shadow-sm"
            placeholder="Add a term (e.g., Supabase, Kubernetes)"
            value={newTerm}
            onChange={(e) => setNewTerm(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addTerm()}
          />
          <button className="btn btn-primary !px-3.5" onClick={addTerm} title="Add term" aria-label="Add term">
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
                  className="text-accent hover:text-rose-500 transition-colors cursor-pointer ml-1"
                  title="Remove term"
                  aria-label={`Remove term ${t.term}`}
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </span>
            ))}
          </div>
        ) : (
          <p className="text-[11.5px] text-muted italic">No custom terms added yet.</p>
        )}
      </Section>

      <Section
        icon={<Sliders className="w-4 h-4" />}
        title="Custom replacements"
        description='Replace spoken phrases after transcription (e.g. "git hub" → GitHub).'
      >
        <div className="flex gap-2">
          <input
            className="field flex-1"
            placeholder="Before"
            value={newBefore}
            onChange={(e) => setNewBefore(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addReplacement()}
          />
          <input
            className="field flex-1"
            placeholder="After"
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
                <p className="flex-1 min-w-0 text-[13px] text-ink-2 truncate">
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
                  className="icon-btn hover:!bg-rose-500/10 hover:!text-rose-500"
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
          <p className="text-[11.5px] text-muted italic">No replacements yet.</p>
        )}
      </Section>
    </>
  );
};
