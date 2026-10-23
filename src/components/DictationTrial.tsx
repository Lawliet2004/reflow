import { useEffect, useRef, useState } from "react";
import { AppState, InjectionFeedback } from "../types";
import { api, safeListen } from "../services/tauriApi";

interface Source {
  session_id: number;
  dictation: boolean;
  final_text: string;
}

/** Completion requires matching native dictation, delivery and paste evidence. */
export function DictationTrial({
  onSuccess,
  onReset,
  onBusyChange,
}: {
  onSuccess: (mode: "inserted" | "clipboard") => void | Promise<void>;
  onReset: () => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  const [text, setText] = useState("");
  const [state, setState] = useState<AppState>("READY");
  const [listening, setListening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<Source | null>(null);
  const [delivery, setDelivery] = useState<InjectionFeedback | null>(null);
  const [verified, setVerified] = useState(false);
  const [busy, setBusy] = useState(false);
  const confirmed = useRef(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const pasted = useRef("");
  const source = useRef<Source | null>(null);
  const feedback = useRef<InjectionFeedback | null>(null);
  const success = useRef(onSuccess);
  const reset = useRef(onReset);
  const callback = useRef<(s: Source) => void>(() => {});
  const seenRecording = useRef(false);
  const alive = useRef(false);
  const epoch = useRef({ value: 0 });
  useEffect(() => {
    success.current = onSuccess;
    reset.current = onReset;
  }, [onSuccess, onReset]);
  const verify = () => {
    const s = source.current;
    const f = feedback.current;
    if (
      !confirmed.current &&
      s &&
      f?.session_id === s.session_id &&
      f.pasted &&
      !f.fallback_copy &&
      pasted.current.trim() === s.final_text.trim()
    ) {
      confirmed.current = true;
      setVerified(true);
      void Promise.resolve(success.current("inserted")).catch((failure) => {
        if (alive.current) setError(String(failure));
      });
    }
  };
  useEffect(() => {
    onBusyChange?.(busy || ["RECORDING", "PROCESSING", "INJECTING"].includes(state));
    return () => onBusyChange?.(false);
  }, [busy, state, onBusyChange]);
  useEffect(() => {
    callback.current = () => verify();
  });
  useEffect(() => {
    alive.current = true;
    const generation = epoch.current;
    let closed = false;
    const unsubs: (() => void)[] = [];
    const listen = async <T,>(name: string, handler: (payload: T) => void) => {
      const unsub = await safeListen<T>(name, (event) => {
        if (!closed) handler(event);
      });
      if (closed) unsub();
      else unsubs.push(unsub);
    };
    void Promise.all([
      listen<AppState>("app:state-changed", (next) => {
        setState(next);
        if (next === "RECORDING") {
          epoch.current.value++;
          reset.current();
          seenRecording.current = true;
          pasted.current = "";
          source.current = null;
          feedback.current = null;
          setResult(null);
          setDelivery(null);
          setVerified(false);
          confirmed.current = false;
        }
      }),
      listen<Source>("transcript:source", (event) => {
        if (!seenRecording.current || !event.dictation || !event.final_text.trim()) return;
        source.current = event;
        setResult(event);
        callback.current(event);
      }),
      listen<InjectionFeedback>("injection:result", (event) => {
        if (!seenRecording.current || !event.session_id) return;
        feedback.current = event;
        setDelivery(event);
        if (source.current) callback.current(source.current);
      }),
    ])
      .then(() => {
        if (!closed) setListening(true);
      })
      .catch((failure) => {
        if (!closed) setError(`Could not listen for dictation results: ${String(failure)}`);
      });
    return () => {
      alive.current = false;
      generation.value++;
      closed = true;
      unsubs.forEach((unsub) => unsub());
    };
  }, []);
  const record = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    box.current?.focus();
    try {
      if (state === "RECORDING") await api.stopRecording();
      else await api.startRecording("dictate");
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      if (alive.current) setBusy(false);
    }
  };
  const clipboardReady = !verified && result && delivery?.session_id === result.session_id;
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted">
        Focus the box and use your shortcut, or choose Record. Say a sentence, then stop.
      </p>
      <textarea
        ref={box}
        className="field w-full resize-none"
        rows={4}
        aria-label="Try dictation here"
        placeholder="Your dictated sentence appears here…"
        value={text}
        onPaste={(event) => {
          pasted.current = event.clipboardData.getData("text/plain");
          verify();
        }}
        onChange={(event) => setText(event.target.value)}
      />
      <button
        type="button"
        className="btn btn-secondary"
        disabled={!listening || busy || state === "PROCESSING" || state === "INJECTING"}
        onClick={() => {
          void record();
        }}
      >
        {state === "RECORDING" ? "Stop recording" : "Record a sentence"}
      </button>
      <div aria-live="polite">
        {state === "RECORDING" && (
          <p className="text-sm text-accent">Listening. Say a sentence, then stop.</p>
        )}
        {(state === "PROCESSING" || state === "INJECTING") && (
          <p className="text-sm text-muted">Transcribing and inserting…</p>
        )}
        {verified && (
          <p className="text-sm text-success">Dictation was recognized and pasted into this box.</p>
        )}
        {result && !verified && (
          <p className="text-sm text-ink whitespace-pre-wrap">Recognized: {result.final_text}</p>
        )}
        {delivery && !verified && <p className="text-xs text-muted">{delivery.message}</p>}
        {clipboardReady && (
          <div className="space-y-2">
            <p className="text-sm text-muted">
              Automatic insertion was not confirmed. You can finish using Copy, and repair insertion
              in Settings → General.
            </p>
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || ["RECORDING", "PROCESSING", "INJECTING"].includes(state)}
              onClick={async () => {
                const current = epoch.current.value;
                setBusy(true);
                try {
                  await api.copyText(result.final_text);
                  if (!alive.current || current !== epoch.current.value) return;
                  await success.current("clipboard");
                } catch (failure) {
                  if (alive.current)
                    setError(`Could not enable clipboard mode: ${String(failure)}`);
                } finally {
                  if (alive.current) setBusy(false);
                }
              }}
            >
              Copy transcript and use clipboard mode
            </button>
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
