import { useState } from "react";
import { api } from "../services/tauriApi";
import type { RecognitionTest } from "../types";

export function RecognitionCheck({
  ready = true,
  onSuccess,
}: {
  ready?: boolean;
  onSuccess?: () => void;
}) {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<RecognitionTest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const test = async () => {
    setTesting(true);
    setResult(null);
    setError(null);
    try {
      const sample = await api.testRecognition();
      if (!sample.text.trim())
        throw new Error("No speech was recognized. Speak clearly and try again.");
      setResult(sample);
      onSuccess?.();
    } catch (failure) {
      setError(String(failure));
    } finally {
      setTesting(false);
    }
  };
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted">
        Speak for three seconds to check your microphone and speech model together. This sample is
        not saved to history or inserted into another app.
      </p>
      <button className="btn btn-secondary" disabled={!ready || testing} onClick={test}>
        {testing ? "Listening and transcribing…" : "Test recognition"}
      </button>
      <div aria-live="polite">
        {testing && <p className="text-sm text-muted">Speak now, then wait for your transcript.</p>}
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
        {result && (
          <div className="space-y-1">
            <p className="text-sm text-ink whitespace-pre-wrap">{result.text}</p>
            <p className="text-sm text-muted">
              {result.health.assessment} · {result.language || "Auto-detected"}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
