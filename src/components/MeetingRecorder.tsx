import { useCallback, useEffect, useState } from "react";
import { Square, Mic, Monitor, ChevronDown } from "lucide-react";
import { api, isTauri, safeListen } from "../services/tauriApi";
import type { Meeting, MeetingConfig, MeetingOutput, MeetingSources } from "../types/meetings";
import type { IntelligenceTierState } from "../types";
export function meetingTime(ms: number) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 3600)
    .toString()
    .padStart(2, "0")}:${Math.floor((s / 60) % 60)
    .toString()
    .padStart(2, "0")}:${(s % 60).toString().padStart(2, "0")}`;
}
const working = (m: Meeting) => ["transcribing", "summarizing"].includes(m.processing);
function SourceText({ text }: { text: string }) {
  return (
    <div className="whitespace-pre-wrap text-sm">
      {text.split(/(\[segment-\d+\]\(#segment-\d+\))/g).map((part, i) => {
        const match = part.match(/^\[(segment-\d+)\]\(#segment-\d+\)$/);
        return match ? (
          <a key={i} className="text-accent underline" href={`#${match[1]}`}>
            {match[1]}
          </a>
        ) : (
          part
        );
      })}
    </div>
  );
}
export function MeetingRecorder({ enabled }: { enabled: boolean }) {
  const [open, setOpen] = useState(false);
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [sources, setSources] = useState<MeetingSources | null>(null);
  const [models, setModels] = useState<IntelligenceTierState[]>([]);
  const [config, setConfig] = useState<MeetingConfig>({
    microphone: null,
    playback: null,
    limit_hours: 5,
    summary_model: null,
  });
  const [probe, setProbe] = useState<[number, number] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [output, setOutput] = useState<MeetingOutput | null>(null);
  const [quit, setQuit] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const active = meetings.find((m) => ["recording", "saving"].includes(m.capture));
  const processing = meetings.find(working);
  const refresh = useCallback(async () => {
    try {
      setMeetings(await api.listMeetings());
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    let alive = true;
    const update = async () => {
      try {
        const list = await api.listMeetings();
        if (alive) setMeetings(list);
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    void update();
    const timer = setInterval(() => update(), 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    const unlisten = safeListen("meeting:quit-requested", () => {
      setQuit(true);
      setOpen(true);
    }).catch(() => () => {});
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);
  useEffect(() => {
    if (!open) return;
    let alive = true;
    void Promise.all([api.getMeetingSources(), api.getIntelligenceTiers()])
      .then(([sources, tiers]) => {
        if (alive) {
          setSources(sources);
          setModels(tiers.filter((t) => t.installed && t.model_id !== "none"));
          setConfig((c) => ({ ...c, playback: c.playback ?? sources.playback[0]?.id ?? null }));
        }
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [open]);
  const perform = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError("");
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const view = async (id: string) => {
    setSelected(id);
    setOutput(await api.getMeetingOutput(id));
  };
  if (!enabled && meetings.length === 0 && !error) return null;
  return (
    <section className="meeting-recorder" aria-label="Meeting recording">
      <div className="meeting-toolbar">
        <button className="btn btn-ghost" aria-expanded={open} onClick={() => setOpen(!open)}>
          <ChevronDown className="w-4 h-4" />
          {active ? "Recording meeting" : "Meeting recordings"}
        </button>
        {active && (
          <>
            <span className="text-sm tabular-nums" role="timer">
              {meetingTime(active.elapsed_ms)} elapsed ·{" "}
              {meetingTime(Math.max(0, active.config.limit_hours * 3600000 - active.elapsed_ms))}{" "}
              remaining
            </span>
            <button
              className="btn btn-primary"
              disabled={busy}
              onClick={() => perform(() => api.stopMeeting(active.id))}
            >
              <Square className="w-4 h-4" />
              Stop and save
            </button>
          </>
        )}
        {active?.warning && (
          <span role="alert" className="text-sm text-warning">
            {active.warning}
          </span>
        )}
        {processing && (
          <span role="status" className="text-sm text-muted">
            {processing.processing === "transcribing"
              ? `Transcribing · ${Math.round((processing.processed_frames / Math.max(1, processing.saved_frames[0] + processing.saved_frames[1])) * 100)}%`
              : `Summarizing · ${processing.summary_progress} spans saved`}
          </span>
        )}
      </div>
      {quit && (
        <div className="meeting-body" role="alert">
          <p>A meeting is recording. Keep Reflow open or stop and save before quitting.</p>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-secondary" onClick={() => setQuit(false)}>
              Keep recording
            </button>
            <button
              className="btn btn-primary"
              disabled={busy}
              onClick={() =>
                perform(async () => {
                  await api.stopMeeting(active?.id);
                  await api.quit();
                })
              }
            >
              Stop, save and quit
            </button>
          </div>
        </div>
      )}
      {open && (
        <div className="meeting-body">
          <h2 className="font-display text-lg font-medium">Capture a meeting</h2>
          <p className="text-sm text-muted">
            Record your microphone and computer playback on this computer. Playback includes other
            apps. Use headphones to reduce double capture and obtain participants’ permission where
            required.
          </p>
          {!active && (
            <>
              <div className="meeting-fields">
                <label>
                  Microphone
                  <select
                    className="field"
                    value={config.microphone ?? ""}
                    disabled={busy || !!processing}
                    onChange={(e) => setConfig({ ...config, microphone: e.target.value || null })}
                  >
                    <option value="">System microphone at Start</option>
                    {sources?.devices.map((d) => (
                      <option key={d.id} value={d.id} disabled={d.is_available === false}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  System playback
                  <select
                    className="field"
                    value={config.playback ?? ""}
                    disabled={busy || !!processing}
                    onChange={(e) => setConfig({ ...config, playback: e.target.value || null })}
                  >
                    <option value="" disabled>
                      Select playback source
                    </option>
                    {sources?.playback.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Stop after
                  <select
                    className="field"
                    value={config.limit_hours}
                    disabled={busy}
                    onChange={(e) => setConfig({ ...config, limit_hours: Number(e.target.value) })}
                  >
                    {[1, 2, 5].map((h) => (
                      <option key={h} value={h}>
                        {h} {h === 1 ? "hour" : "hours"}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Meeting summary model
                  <select
                    className="field"
                    value={config.summary_model ?? ""}
                    disabled={busy || !!processing}
                    onChange={(e) =>
                      setConfig({ ...config, summary_model: e.target.value || null })
                    }
                  >
                    <option value="">Transcript only</option>
                    <option value="follow_writing">Use summary model from Settings</option>
                    {models.map((m) => (
                      <option key={m.model_id} value={m.model_id}>
                        {m.model_id} · installed
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <p className="text-xs text-muted">
                {sources?.explanation ?? "Checking audio sources…"} Audio is saved temporarily even
                when audio retention is disabled. Capture needs no speech model; processing starts
                after Stop. Meetings cannot be paused.
              </p>
              <button
                className="btn btn-primary"
                disabled={
                  !enabled || !sources?.supported || !config.playback || busy || !!processing
                }
                onClick={() => perform(() => api.startDurableMeeting(config))}
              >
                Record meeting
              </button>
              <button
                className="btn btn-secondary"
                disabled={!sources?.supported || !config.playback || busy || !!processing}
                onClick={() =>
                  perform(async () => {
                    setProbe(await api.probeMeetingSources(config));
                  })
                }
              >
                Test sources for 3 seconds
              </button>
              {probe && (
                <div className="meeting-meters" role="status">
                  <p className="text-sm">
                    Both streams opened. Speak and play audio when testing; a quiet meter can mean
                    silence.
                  </p>
                  {["Microphone", "System playback"].map((label, i) => (
                    <label key={label}>
                      {label}
                      <meter aria-label={`${label} test level`} min="0" max="1" value={probe[i]} />
                    </label>
                  ))}
                </div>
              )}
            </>
          )}
          {active && (
            <div className="meeting-meters">
              {["Microphone", "System playback"].map((label, i) => (
                <label key={label}>
                  {i === 0 ? <Mic className="w-4 h-4" /> : <Monitor className="w-4 h-4" />}
                  {label}
                  <meter min="0" max="1" value={active.levels[i]} aria-label={`${label} level`} />
                  {meetingTime(active.saved_frames[i] / 16)} saved
                </label>
              ))}
              <p className="text-xs text-muted">
                Last durable commit: {meetingTime(active.last_commit_ms)}. Closing this window keeps
                recording in the tray.
              </p>
            </div>
          )}
          {error && (
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
          )}
          <ul className="meeting-list">
            {meetings.map((m) => (
              <li key={m.id}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <button
                    className="text-left text-sm underline underline-offset-2"
                    onClick={() => perform(() => view(m.id))}
                  >
                    {new Date(m.created_at).toLocaleString()} · {meetingTime(m.elapsed_ms)}
                  </button>
                  <span className="text-xs text-muted">
                    {m.capture} · transcript {m.transcription_status || m.processing} · summary{" "}
                    {m.summary_status || "pending"}
                  </span>
                </div>
                {(m.warning || m.error) && (
                  <p role="status" className="text-sm text-warning">
                    {m.warning} {m.error}
                  </p>
                )}
                {m.stop_reason && <p className="text-xs text-muted">{m.stop_reason}</p>}
                <div className="flex flex-wrap gap-2">
                  {!active && !working(m) && (
                    <button
                      className="btn btn-secondary"
                      disabled={busy || !!processing}
                      onClick={() => perform(() => api.processMeeting(m.id, config.summary_model))}
                    >
                      Retry processing / summary
                    </button>
                  )}
                  {working(m) && (
                    <button
                      className="btn btn-secondary"
                      onClick={() => perform(() => api.cancelMeetingProcessing())}
                    >
                      Cancel processing
                    </button>
                  )}
                  {!active && !processing && (
                    <button className="btn btn-ghost text-danger" onClick={() => setDeleting(m.id)}>
                      Delete meeting
                    </button>
                  )}
                </div>
                {deleting === m.id && (
                  <div role="alert">
                    <p>Delete this recording, transcript and summary permanently?</p>
                    <button className="btn btn-secondary" onClick={() => setDeleting(null)}>
                      Keep meeting
                    </button>
                    <button
                      className="btn btn-ghost text-danger"
                      onClick={() =>
                        perform(async () => {
                          await api.deleteMeeting(m.id);
                          setDeleting(null);
                          if (selected === m.id) {
                            setSelected(null);
                            setOutput(null);
                          }
                        })
                      }
                    >
                      Delete permanently
                    </button>
                  </div>
                )}
                {m.gaps.length > 0 && (
                  <details>
                    <summary className="text-sm">Missing capture ranges ({m.gaps.length})</summary>
                    {m.gaps.map((g, i) => (
                      <p className="text-xs text-warning" key={i}>
                        {g.track === 0 ? "Microphone" : "Playback"}: {meetingTime(g.start_ms)}–
                        {meetingTime(g.end_ms)} · {g.reason}
                      </p>
                    ))}
                  </details>
                )}
              </li>
            ))}
          </ul>
          {output && selected && (
            <div className="meeting-output">
              <div className="flex flex-wrap gap-2">
                {["txt", "md", "srt", "vtt"].map((format) => (
                  <button
                    className="btn btn-secondary"
                    key={format}
                    onClick={() =>
                      perform(async () => {
                        const text = await api.exportMeeting(selected, format);
                        const a = document.createElement("a");
                        a.href = URL.createObjectURL(
                          new Blob([text], { type: "text/plain;charset=utf-8" }),
                        );
                        a.download = `meeting-${selected}.${format}`;
                        a.click();
                        URL.revokeObjectURL(a.href);
                      })
                    }
                  >
                    Export {format.toUpperCase()}
                  </button>
                ))}
                <button
                  className="btn btn-secondary"
                  onClick={() =>
                    perform(() => api.copyText(output.segments.map((s) => s.text).join("\n")))
                  }
                >
                  Copy transcript
                </button>
              </div>
              {output.summary && (
                <>
                  <h3 className="font-display text-lg">Summary</h3>
                  <SourceText text={output.summary} />
                </>
              )}
              {output.details && (
                <details>
                  <summary>Detailed facts and source references</summary>
                  <SourceText text={output.details} />
                </details>
              )}
              <h3 className="font-display text-lg">
                {meetings.find((m) => m.id === selected)?.transcription_status === "completed"
                  ? "Full transcript"
                  : "Transcript saved so far"}
              </h3>
              {output.segments.map((s, i) => (
                <p id={`segment-${i}`} key={i} className="text-sm">
                  <a className="text-muted underline" href={`#segment-${i}`}>
                    {meetingTime(s.offset_ms)}
                  </a>{" "}
                  {s.text}
                </p>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
