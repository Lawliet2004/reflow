import { useState } from "react";
import type { AppSettings, ApplicationProfile, TranscriptStyle } from "../../types";

export function ApplicationProfiles({
  settings,
  onUpdateSettings,
}: {
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean>;
}) {
  const profiles = settings.application_profiles ?? [];
  const [process, setProcess] = useState("");
  const [style, setStyle] = useState<TranscriptStyle>("faithful");
  const [terms, setTerms] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async (next: ApplicationProfile[]) => {
    setBusy(true);
    setError(null);
    try {
      if (!(await onUpdateSettings({ application_profiles: next })))
        throw new Error("Settings were not saved. Try again.");
      return true;
    } catch (failure) {
      setError(String(failure));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const add = async () => {
    const name = process.trim();
    if (!name) {
      setError("Enter an application process name, such as code.exe.");
      return;
    }
    if (
      profiles.some(
        (p) =>
          p.process.toLowerCase().replace(/\.exe$/, "") ===
          name.toLowerCase().replace(/\.exe$/, ""),
      )
    ) {
      setError("A profile already exists for that application. Remove it before replacing it.");
      return;
    }
    const glossary = terms
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    if (profiles.length >= 64 || glossary.length > 60 || name.length > 200 || terms.length > 6000) {
      setError("Use at most 64 application profiles and 60 short terms per profile.");
      return;
    }
    const dictionary_terms = glossary.map((line) => {
      const [term, ...preferred] = line.split("=");
      return {
        id: crypto.randomUUID(),
        term: term.trim(),
        preferred_spelling: preferred.join("=").trim() || term.trim(),
        category: name,
      };
    });
    if (dictionary_terms.some((t) => !t.term)) {
      setError("Each dictionary line needs a term before the equals sign.");
      return;
    }
    if (await save([...profiles, { process: name, style, dictionary_terms }])) {
      setProcess("");
      setTerms("");
    }
  };
  return (
    <fieldset disabled={busy} className="space-y-3 border-t border-line pt-4 min-w-0">
      <legend className="text-sm font-semibold text-ink">Application writing profiles</legend>
      <p className="text-sm text-muted">
        Match an exact process name to use its writing style and dictionary. Global preferences
        remain available elsewhere.
      </p>
      {profiles.map((profile) => (
        <div
          key={profile.process}
          className="flex flex-wrap gap-2 items-center justify-between text-sm"
        >
          <span className="break-all">
            {profile.process} · {profile.style} · {profile.dictionary_terms.length} terms
          </span>
          <button
            className="btn btn-ghost"
            onClick={() => save(profiles.filter((p) => p !== profile))}
            aria-label={`Remove profile for ${profile.process}`}
          >
            Remove
          </button>
        </div>
      ))}
      <label className="block text-sm">
        Application process
        <input
          className="field w-full mt-1"
          placeholder="code.exe"
          value={process}
          maxLength={200}
          onChange={(e) => setProcess(e.target.value)}
        />
      </label>
      <label className="block text-sm">
        Application style
        <select
          className="field w-full mt-1"
          value={style}
          onChange={(e) => setStyle(e.target.value as TranscriptStyle)}
        >
          {["faithful", "neutral", "decisive", "email", "chat"].map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      </label>
      <label className="block text-sm">
        Application dictionary
        <textarea
          className="field w-full mt-1"
          rows={3}
          placeholder="One term per line, optionally term=preferred spelling"
          value={terms}
          maxLength={6000}
          onChange={(e) => setTerms(e.target.value)}
        />
      </label>
      <button className="btn btn-secondary" onClick={add}>
        {busy ? "Saving…" : "Add application profile"}
      </button>
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </fieldset>
  );
}
