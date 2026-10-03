import { useState } from "react";
import { AppSettings, Snippet } from "../../types";
import { Section, Toggle } from "./ui";

export function SnippetsPage({
  settings,
  onUpdateSettings,
}: {
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean>;
}) {
  const snippets = settings.snippets ?? [];
  const [draft, setDraft] = useState<Snippet | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async (next: Snippet[], close = false) => {
    setBusy(true);
    setError(null);
    try {
      if (await onUpdateSettings({ snippets: next })) {
        if (close) setDraft(null);
      } else setError("Could not save snippets. Try again.");
    } catch {
      setError("Could not save snippets. Try again.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section
      title="Snippets"
      description="Say a phrase to insert stored text exactly as written. Snippets are saved on this computer."
    >
      <fieldset disabled={busy} className="space-y-4 min-w-0">
        {error && (
          <p role="alert" className="text-danger text-sm">
            {error}
          </p>
        )}
        <button
          className="btn btn-primary"
          onClick={() =>
            setDraft({ id: crypto.randomUUID(), trigger: "", expansion: "", enabled: true })
          }
        >
          Add snippet
        </button>
        {!snippets.length && (
          <p className="text-sm text-muted">
            No snippets yet. Add an address, signature or phrase you use often.
          </p>
        )}
        {snippets.map((snippet) => (
          <div key={snippet.id} className="flex items-start gap-3 border-b border-border pb-3">
            <div className="flex-1 min-w-0">
              <strong className="text-sm">{snippet.trigger}</strong>
              <p className="text-xs text-muted whitespace-pre-wrap break-words">
                {snippet.expansion}
              </p>
            </div>
            <Toggle
              on={snippet.enabled}
              ariaLabel={`Enable ${snippet.trigger}`}
              onChange={(enabled) => {
                void save(snippets.map((s) => (s.id === snippet.id ? { ...s, enabled } : s)));
              }}
            />
            <button className="btn btn-ghost" onClick={() => setDraft({ ...snippet })}>
              Edit
            </button>
            <button
              className="btn btn-ghost"
              aria-label={`Delete ${snippet.trigger}`}
              onClick={() => {
                void save(snippets.filter((s) => s.id !== snippet.id));
              }}
            >
              Delete
            </button>
          </div>
        ))}
        {draft && (
          <form
            className="space-y-3 border border-border rounded-lg p-4"
            onSubmit={(event) => {
              event.preventDefault();
              const snippet = { ...draft, trigger: draft.trigger.trim() };
              void save([...snippets.filter((s) => s.id !== draft.id), snippet], true);
            }}
          >
            <label className="block text-sm">
              Spoken trigger
              <input
                className="field w-full mt-1"
                required
                maxLength={60}
                value={draft.trigger}
                onChange={(e) => setDraft({ ...draft, trigger: e.target.value })}
              />
            </label>
            <label className="block text-sm">
              Expansion
              <textarea
                className="field w-full mt-1 min-h-32"
                required
                maxLength={4000}
                value={draft.expansion}
                onChange={(e) => setDraft({ ...draft, expansion: e.target.value })}
              />
            </label>
            <div className="flex gap-2">
              <button className="btn btn-primary" type="submit">
                Save snippet
              </button>
              <button className="btn btn-ghost" type="button" onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </fieldset>
    </Section>
  );
}
