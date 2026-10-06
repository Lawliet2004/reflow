import { useEffect, useRef, useState } from "react";
import { api } from "../../services/tauriApi";
import type { CalibrationStatus } from "../../types";
import { LanguageOptions } from "../LanguageOptions";

export function CalibrationPanel() {
  const [status, setStatus] = useState<CalibrationStatus | null>(null);
  const [reference, setReference] = useState("");
  const [language, setLanguage] = useState("en");
  const [seconds, setSeconds] = useState(7);
  const [running, setRunning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestPending = useRef(false);
  useEffect(() => {
    let alive = true;
    api
      .getCalibrationStatus()
      .then((value) => {
        if (alive) {
          setStatus(value);
          if (!requestPending.current) setRunning(value.running);
          if (value.reference) setReference(value.reference);
          if (value.language) setLanguage(value.language);
        }
      })
      .catch((failure) => {
        if (alive) setError(String(failure));
      });
    return () => {
      alive = false;
    };
  }, []);
  useEffect(() => {
    if (!running) return;
    let alive = true,
      pending = false;
    const timer = setInterval(() => {
      if (pending) return;
      pending = true;
      api
        .getCalibrationStatus()
        .then((value) => {
          if (alive) {
            setStatus(value);
            if (!requestPending.current) setRunning(value.running);
          }
        })
        .catch((failure) => {
          if (alive) setError(String(failure));
        })
        .finally(() => {
          pending = false;
        });
    }, 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [running]);
  const run = async () => {
    requestPending.current = true;
    setRunning(true);
    setError(null);
    setNotice(null);
    setStatus(null);
    try {
      setStatus(await api.runCalibration({ reference: reference.trim(), language, seconds }));
    } catch (failure) {
      setError(String(failure));
    } finally {
      requestPending.current = false;
      setRunning(false);
    }
  };
  const apply = async () => {
    if (!status?.winner_id) return;
    setApplying(true);
    setError(null);
    try {
      if (!(await api.applyCalibration(status.winner_id)))
        throw new Error("Calibration could not be applied.");
      await api.reloadModel();
      setNotice(
        `Automatic performance now uses the qualified ${status.language || language} result. Speech is reloading; wait for Ready before dictating.`,
      );
    } catch (failure) {
      setError(
        `Could not apply or reload calibration: ${String(failure)}. Retry or reload speech in Performance.`,
      );
    } finally {
      setApplying(false);
    }
  };
  const winner = status?.candidates.find((row) => row.id === status.winner_id && row.eligible);
  return (
    <section aria-label="Hardware calibration" className="space-y-3">
      <h3 className="text-sm font-semibold text-ink">Measure speed on your hardware</h3>
      <p className="text-sm text-muted">
        Enter a short phrase in your language, then read it aloud when you run the test. Installed
        speech and writing models stay resident together. One warmup and three measured runs compare
        total processing time; inaccurate, unstable or memory-heavy results cannot win.
      </p>
      <p className="text-xs text-muted">
        This test can take several minutes. The recording stays in memory and is discarded. It is
        never inserted or added to history. Results qualify this phrase and language; they do not
        guarantee accuracy for every voice or recording.
      </p>
      <fieldset disabled={running || applying} className="space-y-3 min-w-0">
        <label className="block text-sm text-ink">
          Reference transcript
          <textarea
            aria-label="Reference transcript"
            className="field w-full mt-1 resize-y"
            maxLength={2048}
            rows={3}
            value={reference}
            onChange={(event) => setReference(event.target.value)}
            placeholder="The exact words you will say"
          />
        </label>
        <div className="flex flex-wrap gap-3">
          <label className="text-sm text-ink">
            Language
            <select
              aria-label="Calibration language"
              className="field block mt-1"
              value={language}
              onChange={(event) => setLanguage(event.target.value)}
            >
              <LanguageOptions includeAuto={false} />
            </select>
          </label>
          <label className="text-sm text-ink">
            Recording seconds
            <input
              aria-label="Recording seconds"
              className="field block mt-1 w-24"
              type="number"
              min={3}
              max={10}
              value={seconds}
              onChange={(event) => setSeconds(Number(event.target.value))}
            />
          </label>
        </div>
        <button
          className="btn btn-primary"
          disabled={!reference.trim() || seconds < 3 || seconds > 10 || running || applying}
          onClick={run}
        >
          Run calibration
        </button>
      </fieldset>
      {running && (
        <div className="space-y-2">
          <p role="status" className="text-sm text-ink">
            {status?.phase || "Recording sample — speak your reference now"}
          </p>
          <button
            className="btn btn-ghost"
            onClick={() => api.cancelCalibration().catch((failure) => setError(String(failure)))}
          >
            Cancel calibration
          </button>
        </div>
      )}
      {(error || status?.error) && (
        <p role="alert" className="text-sm text-danger break-words">
          {error || status?.error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-sm text-ink">
          {notice}
        </p>
      )}
      {Boolean(status?.candidates.length) && (
        <ul className="divide-y divide-line">
          {status?.candidates.map((row) => (
            <li key={row.id} className="py-3 space-y-1">
              <p className="text-sm font-medium text-ink">{row.label}</p>
              <p className="text-xs text-muted">
                Median {Math.round(row.median_ms)} ms · p95 {Math.round(row.p95_ms)} ms · word error{" "}
                {(row.wer * 100).toFixed(1)}% · character error {(row.cer * 100).toFixed(1)}%
              </p>
              {row.error && <p className="text-xs text-danger">{row.error}</p>}
              {row.transcript && (
                <details className="text-xs text-muted">
                  <summary>Compare recognized and written text</summary>
                  <p className="whitespace-pre-wrap mt-2">Recognized: {row.transcript}</p>
                  <p className="whitespace-pre-wrap mt-1">Written: {row.output}</p>
                </details>
              )}
            </li>
          ))}
        </ul>
      )}
      {winner && !running && (
        <button className="btn btn-primary" disabled={applying} onClick={apply}>
          {applying ? "Applying and reloading…" : "Apply fastest qualified settings"}
        </button>
      )}
    </section>
  );
}
