import React from "react";
import {
  AppSettings,
  AppTheme,
  AccentColor,
  HudScale,
  WaveformStyle,
  UIFontScale,
  OverlayPosition,
} from "../../types";
import { Section, Row, Toggle } from "./ui";
import {
  Palette,
  Sun,
  Moon,
  Monitor,
  Sparkles,
  Layers,
  Sliders,
  Check,
  Eye,
  Activity,
} from "lucide-react";
import { Waveform } from "../Waveform";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

interface ThemeOption {
  id: AppTheme;
  title: string;
  desc: string;
  icon: React.ReactNode;
}

const THEME_OPTIONS: ThemeOption[] = [
  {
    id: "system",
    title: "System default",
    desc: "Syncs automatically with your OS preference",
    icon: <Monitor className="w-4 h-4" />,
  },
  {
    id: "light",
    title: "Light",
    desc: "Crisp slate and white surfaces",
    icon: <Sun className="w-4 h-4" />,
  },
  {
    id: "dark",
    title: "Dark",
    desc: "Midnight slate optimized for OLED and low light",
    icon: <Moon className="w-4 h-4" />,
  },
];

interface AccentOption {
  id: AccentColor;
  label: string;
  hex: string;
}

const ACCENT_OPTIONS: AccentOption[] = [
  { id: "sky", label: "Electric Sky", hex: "#0284c7" },
  { id: "indigo", label: "Royal Indigo", hex: "#4f46e5" },
  { id: "emerald", label: "Emerald Green", hex: "#059669" },
  { id: "amber", label: "Sunset Amber", hex: "#d97706" },
  { id: "rose", label: "Crimson Rose", hex: "#e11d48" },
  { id: "violet", label: "Violet Purple", hex: "#7c3aed" },
  { id: "graphite", label: "Carbon Slate", hex: "#475569" },
];

export const AppearancePage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  const activeTheme = settings.app_theme || "system";
  const activeAccent = settings.accent_color || "sky";
  const hudScale = settings.hud_scale || "standard";
  const waveformStyle = settings.waveform_style || "bars";
  const overlayTheme = settings.overlay_theme || "dark";
  const fontScale = settings.ui_font_scale || "normal";

  return (
    <div className="space-y-6">
      {/* Live Preview Panel */}
      <section className="panel p-5 overflow-hidden">
        <div className="flex items-center justify-between pb-3 mb-4 border-b border-line">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-accent-soft border border-accent-border flex items-center justify-center text-accent">
              <Eye className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-[14.5px] font-semibold text-ink tracking-tight">
                Live Theme Preview
              </h2>
              <p className="text-[11.5px] text-muted">
                Changes apply instantly across all Reflow windows
              </p>
            </div>
          </div>
          <span className="chip text-[11px]">
            {activeTheme.toUpperCase()} · {activeAccent.toUpperCase()}
          </span>
        </div>

        {/* Miniature Reflow UI Preview */}
        <div className="p-4 rounded-xl bg-base-2 border border-line space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <span className="w-3 h-3 rounded-full bg-accent animate-pulse" />
              <span className="text-[12.5px] font-semibold text-ink">
                Reflow Dictation
              </span>
              <span className="kbd !text-[10px]">Ctrl+Space</span>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                className="btn btn-primary !py-1.5 !px-3 !text-[11.5px]"
              >
                Primary Button
              </button>
              <button
                type="button"
                className="btn btn-ghost !py-1.5 !px-3 !text-[11.5px]"
              >
                Ghost Action
              </button>
            </div>
          </div>

          {/* Miniature Floating Overlay HUD */}
          <div className="pt-2 border-t border-line">
            <p className="label-micro mb-2">Overlay HUD Preview</p>
            <div
              className={`w-full max-w-[360px] mx-auto h-11 px-4 flex items-center justify-between rounded-full transition-all ${
                overlayTheme === "light"
                  ? "overlay-hud-light"
                  : "overlay-hud overlay-hud-pill"
              }`}
            >
              <div className="flex items-center gap-2.5">
                <span className="relative flex items-center justify-center w-2.5 h-2.5">
                  <span className="absolute inset-0 rounded-full bg-accent/40 animate-ping" />
                  <span className="relative w-2 h-2 rounded-full bg-accent" />
                </span>
                {waveformStyle === "bars" && (
                  <Waveform
                    level={0.65}
                    active
                    barCount={14}
                    height={16}
                    tone={overlayTheme === "light" ? "light" : "dark"}
                  />
                )}
                <span className="text-[11.5px] font-medium text-accent">
                  Listening…
                </span>
              </div>
              <span className="text-[10.5px] opacity-70">
                {settings.overlay_position.replace("_", " ")}
              </span>
            </div>
          </div>
        </div>
      </section>

      {/* Theme Selection */}
      <Section
        icon={<Palette className="w-4 h-4" />}
        title="Theme & color mode"
        description="Choose your preferred visual appearance or follow your system mode"
      >
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          {THEME_OPTIONS.map((item) => {
            const isSelected = activeTheme === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => change("app_theme", item.id)}
                className={`flex flex-col text-left p-3.5 rounded-xl border transition-all cursor-pointer relative ${
                  isSelected
                    ? "border-accent bg-accent-soft shadow-sm ring-1 ring-accent"
                    : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                }`}
              >
                <div className="flex items-center justify-between mb-2">
                  <div
                    className={`w-7 h-7 rounded-lg flex items-center justify-center ${
                      isSelected
                        ? "bg-accent text-white"
                        : "bg-base-2 text-muted"
                    }`}
                  >
                    {item.icon}
                  </div>
                  {isSelected && (
                    <span className="w-4 h-4 rounded-full bg-accent text-white flex items-center justify-center">
                      <Check className="w-2.5 h-2.5 stroke-[3]" />
                    </span>
                  )}
                </div>
                <h3 className="text-[13px] font-semibold text-ink">
                  {item.title}
                </h3>
                <p className="text-[11px] text-muted mt-0.5 leading-relaxed">
                  {item.desc}
                </p>
              </button>
            );
          })}
        </div>

        {/* Accent Color Palette */}
        <div className="pt-4 border-t border-line">
          <p className="text-[13px] font-semibold text-ink mb-1">
            Accent color
          </p>
          <p className="text-[11.5px] text-muted mb-3">
            Customizes buttons, waveform visualizer, focus rings, and active tags
          </p>
          <div className="flex flex-wrap items-center gap-3">
            {ACCENT_OPTIONS.map((c) => {
              const isSelected = activeAccent === c.id;
              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => change("accent_color", c.id)}
                  title={c.label}
                  aria-label={c.label}
                  className={`group relative flex items-center gap-2 px-3 py-1.5 rounded-lg border transition-all cursor-pointer ${
                    isSelected
                      ? "border-accent bg-accent-soft ring-1 ring-accent shadow-xs"
                      : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                  }`}
                >
                  <span
                    className="w-3.5 h-3.5 rounded-full shadow-xs shrink-0 flex items-center justify-center"
                    style={{ backgroundColor: c.hex }}
                  >
                    {isSelected && (
                      <Check className="w-2.5 h-2.5 text-white stroke-[3]" />
                    )}
                  </span>
                  <span className="text-[12px] font-medium text-ink">
                    {c.label}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      </Section>

      {/* Overlay & HUD Styling */}
      <Section
        icon={<Layers className="w-4 h-4" />}
        title="Overlay & HUD appearance"
        description="Configure the floating listening pill that appears while dictating"
      >
        <Row
          label="Overlay position"
          hint="Screen anchor location for the listening HUD"
        >
          <select
            className="field"
            value={settings.overlay_position}
            onChange={(e) =>
              change("overlay_position", e.target.value as OverlayPosition)
            }
          >
            <option value="bottom_center">Bottom center (default)</option>
            <option value="top_center">Top center</option>
            <option value="bottom_right">Bottom right</option>
            <option value="top_right">Top right</option>
          </select>
        </Row>

        <Row
          label="HUD color style"
          hint="Translucent glass theme for the floating indicator"
        >
          <select
            className="field"
            value={overlayTheme}
            onChange={(e) =>
              change("overlay_theme", e.target.value as "dark" | "light" | "auto")
            }
          >
            <option value="dark">Dark OLED glass (recommended)</option>
            <option value="light">Frosted light glass</option>
            <option value="auto">Auto (match app theme)</option>
          </select>
        </Row>

        <Row
          label="Waveform visualizer"
          hint="Style of the live audio visualizer while recording"
        >
          <select
            className="field"
            value={waveformStyle}
            onChange={(e) =>
              change("waveform_style", e.target.value as WaveformStyle)
            }
          >
            <option value="bars">Dynamic multi-bar (22 bars)</option>
            <option value="pulse">Pulse dot only</option>
            <option value="minimal">Minimal / Stealth</option>
          </select>
        </Row>

        <Row
          label="HUD sizing"
          hint="Scale dimension for the floating pill"
        >
          <select
            className="field"
            value={hudScale}
            onChange={(e) => change("hud_scale", e.target.value as HudScale)}
          >
            <option value="compact">Compact (360px)</option>
            <option value="standard">Standard (420px)</option>
            <option value="large">Large (480px)</option>
          </select>
        </Row>
      </Section>

      {/* Interface & Accessibility */}
      <Section
        icon={<Sliders className="w-4 h-4" />}
        title="Interface & accessibility"
        description="Display density and motion comfort settings"
      >
        <Row
          label="UI font scale"
          hint="Adjust text and element sizing throughout the application"
        >
          <select
            className="field"
            value={fontScale}
            onChange={(e) =>
              change("ui_font_scale", e.target.value as UIFontScale)
            }
          >
            <option value="compact">Compact (90%)</option>
            <option value="normal">Default (100%)</option>
            <option value="roomy">Roomy (110%)</option>
          </select>
        </Row>

        <Row
          label="Reduce motion"
          hint="Disables heavy pulse, breathe, and waveform animations for battery and comfort"
        >
          <Toggle
            on={Boolean(settings.reduce_motion)}
            onChange={(v) => change("reduce_motion", v)}
            ariaLabel="Reduce motion"
          />
        </Row>
      </Section>
    </div>
  );
};
