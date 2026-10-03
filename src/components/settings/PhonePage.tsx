import React, { useEffect, useRef, useState } from "react";
import { AppSettings, ApiStatus } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row, Toggle, qrSrc } from "./ui";
import { Check, Copy, RefreshCw, Smartphone } from "lucide-react";

type PairView = "qr" | "code" | "link";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const PhonePage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [apiStatus, setApiStatus] = useState<ApiStatus | null>(null);
  const [copiedPair, setCopiedPair] = useState<"code" | "uri" | null>(null);
  const [view, setView] = useState<PairView>("qr");
  const [error, setError] = useState<string | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const statusEpoch = useRef(0);

  useEffect(() => {
    let cancelled = false;
    let pending = false;
    const tick = () => {
      if (pending) return;
      pending = true;
      const epoch = statusEpoch.current;
      api
        .getApiStatus()
        .then((status) => {
          if (!cancelled && epoch === statusEpoch.current) {
            setApiStatus(status);
            setStatusError(null);
          }
        })
        .catch(() => {
          if (!cancelled && epoch === statusEpoch.current)
            setStatusError("Could not refresh phone status. Retrying shortly.");
        })
        .finally(() => {
          pending = false;
        });
    };
    tick();
    const id = setInterval(tick, 2500);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [settings.api_enabled, settings.api_bind, settings.api_port]);

  const rotate = async () => {
    statusEpoch.current += 1;
    setWorking("rotate");
    setError(null);
    try {
      setApiStatus(await api.rotatePairingCode());
    } catch {
      setError("Could not rotate the pairing code. Please try again.");
    } finally {
      statusEpoch.current += 1;
      setWorking(null);
    }
  };

  const revoke = async (id: string) => {
    statusEpoch.current += 1;
    setWorking(id);
    setError(null);
    try {
      await api.revokeApiDevice(id);
      setApiStatus((status) =>
        status
          ? { ...status, devices: status.devices.filter((device) => device.id !== id) }
          : status,
      );
    } catch {
      setError("Could not revoke this phone. It is still paired; please try again.");
    } finally {
      statusEpoch.current += 1;
      setWorking(null);
    }
  };

  const setPermission = async (
    id: string,
    permissions: { stream: boolean; history: boolean; injection: boolean },
  ) => {
    statusEpoch.current += 1;
    setWorking(id);
    setError(null);
    try {
      if (!(await api.setApiDevicePermissions(id, permissions)))
        throw new Error("Device no longer paired");
      setApiStatus((status) =>
        status
          ? {
              ...status,
              devices: status.devices.map((device) =>
                device.id === id ? { ...device, permissions } : device,
              ),
            }
          : status,
      );
    } catch (failure) {
      setError(`Could not save phone permissions: ${String(failure)}`);
    } finally {
      statusEpoch.current += 1;
      setWorking(null);
    }
  };

  const copyPair = async (kind: "code" | "uri", value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedPair(kind);
    } catch {
      setError("Could not copy the pairing information. Please try again.");
    }
  };

  useEffect(() => {
    if (!copiedPair) return;
    const timer = setTimeout(() => setCopiedPair(null), 1400);
    return () => clearTimeout(timer);
  }, [copiedPair]);

  return (
    <Section icon={<Smartphone className="w-4 h-4" />} title="Phone companion">
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {statusError && (
        <p role="alert" className="text-sm text-danger">
          {statusError}
        </p>
      )}
      <Row label="Enable LAN API" hint="Let a phone on your network stream audio to this computer">
        <Toggle
          on={settings.api_enabled}
          onChange={(v) => onUpdateSettings({ api_enabled: v })}
          ariaLabel="Enable LAN API"
        />
      </Row>

      <Row label="Bind" hint="Localhost is this PC only. LAN is your Wi-Fi.">
        <select
          className="field"
          value={settings.api_bind}
          onChange={(e) =>
            onUpdateSettings({ api_bind: e.target.value as AppSettings["api_bind"] })
          }
        >
          <option value="lan">LAN</option>
          <option value="localhost">Localhost</option>
        </select>
      </Row>

      <Row label="Port">
        <input
          className="field !w-[100px]"
          type="number"
          min={1024}
          max={65535}
          value={settings.api_port}
          onChange={(e) => onUpdateSettings({ api_port: Number(e.target.value) || 7840 })}
        />
      </Row>

      {apiStatus?.warning && (
        <p className="text-xs text-muted leading-relaxed">{apiStatus.warning}</p>
      )}

      {settings.api_enabled && apiStatus && (
        <div className="rounded-xl border border-line bg-surface-2 p-4 space-y-3">
          <div className="flex items-center gap-1.5 self-start">
            {(["qr", "code", "link"] as PairView[]).map((v) => (
              <button
                key={v}
                onClick={() => setView(v)}
                className={`px-2.5 py-1 rounded-full text-xs font-semibold transition-colors cursor-pointer ${
                  view === v
                    ? "bg-accent-soft text-accent border border-accent-border"
                    : "bg-surface text-muted border border-line hover:text-ink hover:bg-base-2"
                }`}
              >
                {v === "qr" ? "QR" : v === "code" ? "Code" : "Link"}
              </button>
            ))}
          </div>

          {view === "qr" && apiStatus.qr_svg && (
            <div className="flex items-start gap-4">
              <img
                src={qrSrc(apiStatus.qr_svg)}
                alt="Pairing QR"
                className="w-[160px] h-[160px] rounded-lg bg-white p-2 border border-line"
              />
              <div className="min-w-0 flex-1 space-y-2 text-sm text-muted">
                <p className="text-ink">Open the Reflow app on your phone and scan this code.</p>
                {apiStatus.pair_uri && (
                  <p className="text-muted">
                    Or paste the link in <span className="kbd !text-2xs">Pair</span>.
                  </p>
                )}
                <button
                  className="btn btn-ghost !py-1.5 !px-2.5 !text-xs"
                  onClick={rotate}
                  disabled={working !== null}
                >
                  <RefreshCw className="w-3.5 h-3.5" />
                  Rotate
                </button>
              </div>
            </div>
          )}

          {view === "code" && (
            <div className="space-y-2">
              <p className="text-xs text-muted">Pairing code</p>
              <p className="text-2xl leading-none font-semibold tracking-[0.18em] text-ink">
                {apiStatus.pairing_code ?? "——————"}
              </p>
              {apiStatus.pairing_expires_in_sec != null && (
                <p className="text-xs text-muted">Expires in {apiStatus.pairing_expires_in_sec}s</p>
              )}
              <div className="flex flex-wrap gap-2 pt-1">
                {apiStatus.pairing_code && (
                  <button
                    className="btn btn-ghost !py-1.5 !px-2.5 !text-xs"
                    onClick={() => copyPair("code", apiStatus.pairing_code!)}
                  >
                    {copiedPair === "code" ? (
                      <Check className="w-3.5 h-3.5 text-success" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                    Copy code
                  </button>
                )}
                <button
                  className="btn btn-ghost !py-1.5 !px-2.5 !text-xs"
                  onClick={rotate}
                  disabled={working !== null}
                >
                  <RefreshCw className="w-3.5 h-3.5" />
                  Rotate
                </button>
              </div>
            </div>
          )}

          {view === "link" && apiStatus.pair_uri && (
            <div>
              <p className="text-xs text-muted mb-1">Pair link</p>
              <div className="flex items-center gap-2">
                <code className="field flex-1 !text-xs truncate">{apiStatus.pair_uri}</code>
                <button
                  className="icon-btn border border-line"
                  onClick={() => copyPair("uri", apiStatus.pair_uri!)}
                  title="Copy pair link"
                  aria-label="Copy pair link"
                >
                  {copiedPair === "uri" ? (
                    <Check className="w-3.5 h-3.5 text-success" />
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </button>
              </div>
            </div>
          )}

          {apiStatus.listen_addrs.length > 0 && (
            <p className="text-xs text-muted">
              Listening on {apiStatus.listen_addrs.join(", ")}:{apiStatus.port}
              {apiStatus.running ? "" : " · server starting…"}
            </p>
          )}
          {apiStatus.certificate_sha256 && (
            <div className="space-y-2">
              <p className="text-xs text-muted">
                Desktop certificate fingerprint (SHA-256). Copy this into manual phone pairing, or
                use the full pairing link.
              </p>
              <code className="block text-xs text-ink break-all select-all">
                {apiStatus.certificate_sha256}
              </code>
              <button
                className="btn btn-ghost"
                onClick={() => copyPair("uri", apiStatus.certificate_sha256!)}
              >
                Copy fingerprint
              </button>
            </div>
          )}
        </div>
      )}

      <div>
        <p className="text-sm font-medium text-ink mb-2">Paired devices</p>
        {(apiStatus?.devices ?? []).length === 0 ? (
          <p className="text-xs text-muted">No phones paired yet.</p>
        ) : (
          <div className="divide-y divide-line rounded-xl border border-line overflow-hidden">
            {(apiStatus?.devices ?? []).map((device) => (
              <div key={device.id} className="flex items-center gap-3 px-3 py-2.5">
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-ink truncate">{device.name}</p>
                  <p className="text-2xs text-muted">{device.created_at}</p>
                  <div className="flex flex-wrap gap-3 pt-2 text-xs text-muted">
                    {(
                      [
                        ["stream", "Stream audio"],
                        ["history", "Read history"],
                        ["injection", "Paste on desktop"],
                      ] as const
                    ).map(([key, label]) => {
                      const permissions = device.permissions ?? {
                        stream: true,
                        history: false,
                        injection: false,
                      };
                      return (
                        <label key={key} className="flex items-center gap-1.5">
                          <input
                            type="checkbox"
                            checked={permissions[key]}
                            disabled={working !== null}
                            aria-label={`${label} for ${device.name}`}
                            onChange={(event) =>
                              setPermission(device.id, {
                                ...permissions,
                                [key]: event.target.checked,
                              })
                            }
                          />
                          {label}
                        </label>
                      );
                    })}
                  </div>
                </div>
                <button
                  className="btn btn-danger !py-1 !px-2.5 !text-xs"
                  onClick={() => revoke(device.id)}
                  disabled={working !== null}
                >
                  Revoke
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </Section>
  );
};
