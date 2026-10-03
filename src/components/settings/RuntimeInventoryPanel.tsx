import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../services/tauriApi";
import type { RuntimeEntry } from "../../types";

export function RuntimeInventoryPanel({ busy = false }: { busy?: boolean }) {
  const [entries, setEntries] = useState<RuntimeEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<"repair" | "rollback" | "reload" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [needsReload, setNeedsReload] = useState(false);
  const alive = useRef(true);
  const epoch = useRef(0);
  const operation = useRef(false);

  const refresh = useCallback(async () => {
    const request = ++epoch.current;
    setLoading(true);
    setError(null);
    try {
      const inventory = await api.getRuntimeInventory();
      if (alive.current && request === epoch.current) setEntries(inventory);
    } catch (failure) {
      if (alive.current && request === epoch.current) {
        setError(`Could not inspect runtimes: ${String(failure)}`);
      }
    } finally {
      if (alive.current && request === epoch.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    if (!busy)
      queueMicrotask(() => {
        if (alive.current) refresh();
      });
    return () => {
      alive.current = false;
    };
  }, [busy, refresh]);

  const run = async (action: "repair" | "rollback" | "reload") => {
    if (busy || operation.current) return;
    operation.current = true;
    ++epoch.current;
    setLoading(false);
    setPending(action);
    setError(null);
    setNotice(null);
    let runtimeChanged = false;
    try {
      if (action === "rollback") {
        const inventory = await api.rollbackRuntime();
        runtimeChanged = true;
        if (alive.current) setEntries(inventory);
      } else if (action === "repair") {
        await api.repairRuntime();
        runtimeChanged = true;
        const inventory = await api.getRuntimeInventory();
        if (alive.current) setEntries(inventory);
      }
      if (alive.current) setNeedsReload(true);
      await api.reloadModel();
      if (alive.current) {
        setNeedsReload(false);
        setNotice(
          action === "rollback"
            ? "Previous runtime restored. Speech model reloaded."
            : action === "repair"
              ? "Runtime repaired. Speech model reloaded."
              : "Speech model reloaded.",
        );
      }
    } catch (failure) {
      if (alive.current) {
        setNeedsReload(runtimeChanged || action === "reload");
        setError(
          runtimeChanged
            ? `Runtime changed, but speech could not reload: ${String(failure)}. Use Reload speech model to retry.`
            : String(failure),
        );
      }
    } finally {
      operation.current = false;
      if (alive.current) setPending(null);
    }
  };

  const disabled = busy || pending !== null || loading;
  const rollback = entries.find(
    (entry) => !entry.active && entry.rollback_available && entry.healthy,
  );
  const external = entries.some((entry) => entry.active && entry.id === "custom");

  return (
    <section
      aria-label="Runtime inventory"
      className="space-y-3 border-t border-line pt-4"
      aria-busy={loading || pending !== null || busy}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-ink">Runtime inventory</h3>
        <button className="btn btn-ghost" type="button" disabled={disabled} onClick={refresh}>
          Refresh runtimes
        </button>
      </div>
      <p className="text-sm text-muted">
        Repair or rollback stops the model runtimes, then reloads your speech model. Wait for it to
        finish before dictating.
      </p>
      {error && (
        <p role="alert" className="text-sm text-danger break-words">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-sm text-ink">
          {notice}
        </p>
      )}
      {loading ? (
        <p role="status" className="text-sm text-muted">
          Inspecting runtimes…
        </p>
      ) : entries.length === 0 ? (
        <p className="text-sm text-muted">
          No managed runtime is installed. Repair downloads and verifies one for this hardware.
        </p>
      ) : (
        <ul className="divide-y divide-line">
          {entries.map((entry) => (
            <li key={entry.id} className="py-3 space-y-1">
              <p className="text-sm font-medium text-ink">
                {entry.active ? "Current" : entry.rollback_available ? "Previous" : "Saved"} ·{" "}
                {entry.id === "custom" ? "External" : entry.healthy ? "Verified" : "Unverified"}
              </p>
              <p className="text-sm text-muted">
                {entry.kind} · <span>{entry.version}</span>
              </p>
              <p className="text-xs text-muted break-all font-mono">{entry.binary_path}</p>
              {entry.error && <p className="text-sm text-danger break-words">{entry.error}</p>}
            </li>
          ))}
        </ul>
      )}
      {external && (
        <p className="text-sm text-muted">
          A custom launcher is active. Its checksum is not verified here. Remove REFLOW_LLAMA_BIN to
          repair or roll back the managed runtime.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          className="btn btn-ghost"
          type="button"
          disabled={disabled || external}
          onClick={() => run("repair")}
        >
          {pending === "repair" ? "Repairing runtime…" : "Repair runtime"}
        </button>
        {rollback && (
          <button
            className="btn btn-ghost"
            type="button"
            disabled={disabled || external}
            onClick={() => run("rollback")}
          >
            {pending === "rollback" ? "Restoring runtime…" : "Use previous runtime"}
          </button>
        )}
        {needsReload && (
          <button
            className="btn btn-primary"
            type="button"
            disabled={disabled}
            onClick={() => run("reload")}
          >
            {pending === "reload" ? "Reloading speech…" : "Reload speech model"}
          </button>
        )}
      </div>
    </section>
  );
}
