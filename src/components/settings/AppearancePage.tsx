import React, { useEffect, useRef, useState } from "react";
import {
  APPEARANCE_DEFAULTS,
  AppSettings,
  AppState,
  OverlayPosition,
  StreamingTranscriptPayload,
  SurfaceTone,
  pickAppearance,
} from "../../types";
import { Section, Row, Toggle } from "./ui";
import { Overlay } from "../Overlay";
import { contrastRatio, getResolvedTheme } from "../../utils/theme";
import { Check, Mic, Monitor, Palette, Pipette, RotateCcw, Type } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

/* Thumbnail palettes. Mirrors the tone tokens in globals.css. */
type Paper = { base: string; side: string; surface: string; ink: string };
const PAPER: Record<SurfaceTone, { light: Paper; dark: Paper }> = {
  warm: {
    light: { base: "#f4efe6", side: "#ebe4d8", surface: "#fbf8f2", ink: "#29231e" },
    dark: { base: "#1a1714", side: "#24201b", surface: "#25211c", ink: "#f2ece2" },
  },
  neutral: {
    light: { base: "#f2f2f0", side: "#e8e8e5", surface: "#fbfbfa", ink: "#232322" },
    dark: { base: "#161616", side: "#1f1f1f", surface: "#222222", ink: "#ededec" },
  },
  cool: {
    light: { base: "#eef1f4", side: "#e3e8ed", surface: "#f9fafc", ink: "#1e252d" },
    dark: { base: "#13171b", side: "#1b2026", surface: "#1e2329", ink: "#e9eef3" },
  },
};

/* Swatch inks. Mirrors the `data-accent` tokens in globals.css. */
const ACCENTS = [
  { id: "sky", label: "Blue ink", light: "#2a6690", dark: "#86b6d8" },
  { id: "indigo", label: "Indigo ink", light: "#4a4aa0", dark: "#a3a3e2" },
  { id: "emerald", label: "Green ink", light: "#2c6e52", dark: "#86c7a6" },
  { id: "amber", label: "Sepia", light: "#95570f", dark: "#e3ae62" },
  { id: "rose", label: "Red pencil", light: "#a63245", dark: "#e8909c" },
  { id: "violet", label: "Violet ink", light: "#65479b", dark: "#b9a0e2" },
  { id: "graphite", label: "Graphite", light: "#5c534a", dark: "#b8ad9e" },
] as const;

const POSITIONS: { id: OverlayPosition; label: string }[] = [
  { id: "top_left", label: "Top left" },
  { id: "top_center", label: "Top center" },
  { id: "top_right", label: "Top right" },
  { id: "bottom_left", label: "Bottom left" },
  { id: "bottom_center", label: "Bottom center" },
  { id: "bottom_right", label: "Bottom right" },
];

const PREVIEW_PHASES: { id: AppState; label: string }[] = [
  { id: "RECORDING", label: "Listening" },
  { id: "PROCESSING", label: "Transcribing" },
  { id: "READY", label: "Done" },
];

interface Option<T> {
  value: T;
  label: React.ReactNode;
  name?: string;
}

/** A radio group with roving focus. Renders as a segmented control unless styled. */
function Choice<T extends string | number>({
  label,
  value,
  options,
  onChange,
  className = "segmented-control",
}: {
  label: string;
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const current = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const onKeyDown = (e: React.KeyboardEvent) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    if (!step) return;
    e.preventDefault();
    const next = (current + step + options.length) % options.length;
    onChange(options[next].value);
    ref.current?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  };
  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={label}
      tabIndex={-1}
      className={className}
      onKeyDown={onKeyDown}
    >
      {options.map((o, i) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          aria-label={o.name}
          tabIndex={i === current ? 0 : -1}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A labelled block for controls that need the full row width. */
const Block: React.FC<{ label: string; hint?: string; children: React.ReactNode }> = ({
  label,
  hint,
  children,
}) => (
  <div className="look-block">
    <p className="text-sm font-medium text-ink">{label}</p>
    {hint && <p className="mt-0.5 text-xs leading-relaxed text-muted">{hint}</p>}
    <div className="mt-3">{children}</div>
  </div>
);

const Thumb: React.FC<{ paper: Paper; className?: string }> = ({ paper, className }) => (
  <span
    className={`look-thumb ${className ?? ""}`}
    style={
      {
        "--t-base": paper.base,
        "--t-side": paper.side,
        "--t-surface": paper.surface,
        "--t-ink": paper.ink,
      } as React.CSSProperties
    }
    aria-hidden
  >
    <span className="look-thumb-sheet">
      <span className="look-thumb-seal" />
      <i />
      <i />
      <i />
    </span>
  </span>
);

const TileLabel: React.FC<{ name: string; hint: string; thumb: React.ReactNode }> = ({
  name,
  hint,
  thumb,
}) => (
  <>
    {thumb}
    <span className="look-tile-name">
      {name}
      <Check className="look-tile-check" aria-hidden />
    </span>
    <span className="look-tile-hint">{hint}</span>
  </>
);

const Corner: React.FC<{ r: number }> = ({ r }) => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden className="inline-block mr-1.5">
    <path
      d={`M1.5 11V${1.5 + r}Q1.5 1.5 ${1.5 + r} 1.5H11`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
    />
  </svg>
);

export const AppearancePage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const a = pickAppearance(settings);
  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);
  return (
    <div className="space-y-6">
      <ColourSection
        a={a}
        mode={getResolvedTheme(a.app_theme)}
        change={change}
        update={onUpdateSettings}
      />

      <Section
        icon={<Type className="w-4 h-4" />}
        title="Type & layout"
        description="How words are set and how much room the interface gives them."
      >
        <div className="proof" aria-label="Preview of your type settings" role="figure">
          <h3>Notes from Tuesday</h3>
          <p>
            Speak freely. Reflow sets what you say in this face, so drafts, notes and history read
            the way you like to read.
          </p>
          <div className="proof-actions" aria-hidden>
            <span className="btn btn-primary">Insert</span>
            <span className="btn btn-secondary">Copy</span>
            <span className="chip">Polished</span>
          </div>
        </div>

        <Block label="Reading face" hint="Used for transcripts, notes and history.">
          <Choice
            label="Reading face"
            className="look-grid"
            value={a.reading_font}
            onChange={(v) => change("reading_font", v)}
            options={[
              {
                value: "serif",
                name: "Book serif",
                label: <FontTile face="book" name="Book serif" hint="Warm, for long reading" />,
              },
              {
                value: "sans",
                name: "System sans",
                label: <FontTile face="sans" name="System sans" hint="Clean and familiar" />,
              },
              {
                value: "mono",
                name: "Monospace",
                label: <FontTile face="mono" name="Monospace" hint="Even, for code and notes" />,
              },
            ]}
          />
        </Block>

        <Row label="Headings" hint="Page and section titles.">
          <Choice
            label="Heading face"
            value={a.heading_font}
            onChange={(v) => change("heading_font", v)}
            options={[
              {
                value: "serif",
                label: <span style={{ fontFamily: "var(--face-book)" }}>Book serif</span>,
              },
              {
                value: "sans",
                label: <span style={{ fontFamily: "var(--face-sans)" }}>System sans</span>,
              },
            ]}
          />
        </Row>

        <Row label="Text size" hint="Scales text and controls everywhere.">
          <Choice
            label="Text size"
            value={a.ui_font_scale}
            onChange={(v) => change("ui_font_scale", v)}
            options={[
              { value: "compact", label: "Small" },
              { value: "normal", label: "Default" },
              { value: "roomy", label: "Large" },
              { value: "large", label: "Larger" },
            ]}
          />
        </Row>

        <Row label="Density" hint="Spacing around sections, rows and lists.">
          <Choice
            label="Density"
            value={a.ui_density}
            onChange={(v) => change("ui_density", v)}
            options={[
              { value: "compact", label: "Compact" },
              { value: "comfortable", label: "Comfortable" },
              { value: "spacious", label: "Spacious" },
            ]}
          />
        </Row>

        <Row label="Corners" hint="Roundness of sheets, buttons and fields.">
          <Choice
            label="Corners"
            value={a.corner_style}
            onChange={(v) => change("corner_style", v)}
            options={[
              {
                value: "sharp",
                label: (
                  <>
                    <Corner r={1.5} />
                    Sharp
                  </>
                ),
              },
              {
                value: "soft",
                label: (
                  <>
                    <Corner r={4} />
                    Soft
                  </>
                ),
              },
              {
                value: "round",
                label: (
                  <>
                    <Corner r={7} />
                    Round
                  </>
                ),
              },
            ]}
          />
        </Row>
      </Section>

      <PillSection settings={settings} a={a} change={change} />

      <Section title="Motion">
        <Row
          label="Reduce motion"
          hint="Stills the record ring, page fades and pill animations. Your system setting is always respected."
        >
          <Toggle
            on={a.reduce_motion}
            onChange={(v) => change("reduce_motion", v)}
            ariaLabel="Reduce motion"
          />
        </Row>
      </Section>

      <ResetRow onReset={() => onUpdateSettings(APPEARANCE_DEFAULTS)} />
    </div>
  );
};

type Change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void;
type A = ReturnType<typeof pickAppearance>;

const FontTile: React.FC<{ face: "book" | "sans" | "mono"; name: string; hint: string }> = ({
  face,
  name,
  hint,
}) => (
  <TileLabel
    name={name}
    hint={hint}
    thumb={
      <span className="font-thumb" style={{ fontFamily: `var(--face-${face})` }} aria-hidden>
        <span className="font-thumb-glyph">Aa</span>
        <span className="font-thumb-line">One less thought to lose.</span>
      </span>
    }
  />
);

const ColourSection: React.FC<{
  a: A;
  mode: "light" | "dark";
  change: Change;
  update: (s: Partial<AppSettings>) => void;
}> = ({ a, mode, change, update }) => {
  const papers = PAPER[a.surface_tone];
  const [draft, setDraft] = useState<string | null>(null);
  const colorRef = useRef<HTMLInputElement>(null);
  const custom = draft ?? a.accent_custom;

  // The native `change` fires once the picker closes; `input` (React's
  // onChange) fires while dragging and only previews, so a drag is one save.
  useEffect(() => {
    const el = colorRef.current;
    if (!el) return;
    const commit = () => {
      change("accent_custom", el.value);
      setDraft(null);
    };
    el.addEventListener("change", commit);
    return () => el.removeEventListener("change", commit);
  });

  const ratio = contrastRatio(custom, papers[mode].surface);

  return (
    <Section
      icon={<Palette className="w-4 h-4" />}
      title="Colour & paper"
      description="The light you work in, the paper under your words and the ink that marks what's active."
    >
      <Block label="Theme">
        <Choice
          label="Theme"
          className="look-grid"
          value={a.app_theme}
          onChange={(v) => change("app_theme", v)}
          options={[
            {
              value: "system",
              name: "Match system",
              label: (
                <TileLabel
                  name="Match system"
                  hint="Follows Windows light and dark"
                  thumb={
                    <span className="look-thumb-pair">
                      <Thumb paper={papers.light} />
                      <Thumb paper={papers.dark} className="look-thumb-half" />
                    </span>
                  }
                />
              ),
            },
            {
              value: "light",
              name: "Light",
              label: (
                <TileLabel
                  name="Light"
                  hint="Paper by daylight"
                  thumb={<Thumb paper={papers.light} />}
                />
              ),
            },
            {
              value: "dark",
              name: "Dark",
              label: (
                <TileLabel
                  name="Dark"
                  hint="Lamp-lit, for evenings"
                  thumb={<Thumb paper={papers.dark} />}
                />
              ),
            },
          ]}
        />
      </Block>

      <Block label="Paper" hint="The tint of every surface.">
        <Choice
          label="Paper"
          className="look-grid"
          value={a.surface_tone}
          onChange={(v) => change("surface_tone", v)}
          options={(
            [
              ["warm", "Warm", "Cream, like a notebook"],
              ["neutral", "Neutral", "Plain white and grey"],
              ["cool", "Cool", "A faint blue slate"],
            ] as const
          ).map(([value, name, hint]) => ({
            value,
            name,
            label: (
              <TileLabel
                name={name}
                hint={hint}
                thumb={<Thumb paper={PAPER[value][mode]} className="look-thumb-sm" />}
              />
            ),
          }))}
        />
      </Block>

      <Block
        label="Accent ink"
        hint="Focus rings, links, selection, the record seal and the listening pill."
      >
        <Choice
          label="Accent ink"
          className="swatches"
          value={a.accent_color}
          onChange={(v) => {
            // First switch to custom starts from the ink in use, not an arbitrary blue.
            const from = ACCENTS.find((c) => c.id === a.accent_color);
            update(
              v === "custom" && from && a.accent_custom === APPEARANCE_DEFAULTS.accent_custom
                ? { accent_color: v, accent_custom: from[mode] }
                : { accent_color: v },
            );
          }}
          options={[
            ...ACCENTS.map((c) => ({
              value: c.id,
              name: c.label,
              label: (
                <span
                  className="swatch"
                  style={{ "--sw": c[mode] } as React.CSSProperties}
                  title={c.label}
                >
                  <Check aria-hidden />
                </span>
              ),
            })),
            {
              value: "custom" as const,
              name: "Custom ink",
              label: (
                <span
                  className="swatch swatch-custom"
                  style={{ "--sw": custom } as React.CSSProperties}
                  title="Custom ink"
                >
                  <Pipette aria-hidden />
                </span>
              ),
            },
          ]}
        />
        <p className="mt-2.5 text-xs text-muted">
          {a.accent_color === "custom"
            ? "Custom ink"
            : ACCENTS.find((c) => c.id === a.accent_color)?.label}
        </p>
        {a.accent_color === "custom" && (
          <div className="custom-ink">
            <label className="custom-ink-pick">
              <input
                ref={colorRef}
                type="color"
                value={custom}
                onChange={(e) => {
                  setDraft(e.target.value);
                  document.documentElement.style.setProperty("--accent-custom", e.target.value);
                }}
              />
              <span>
                Pick ink <code>{custom.toUpperCase()}</code>
              </span>
            </label>
            <p className={ratio >= 4.5 ? "text-success" : "text-warning"} role="status">
              {ratio.toFixed(1)}:1 on this paper ·{" "}
              {ratio >= 4.5
                ? "readable as text"
                : mode === "dark"
                  ? "too dim for links; try a lighter ink"
                  : "too faint for links; try a deeper ink"}
            </p>
          </div>
        )}
      </Block>

      <Row
        label="Window material"
        hint="Mica lets your desktop tint the title bar and sidebar on Windows 11."
      >
        <Choice
          label="Window material"
          value={a.window_material}
          onChange={(v) => change("window_material", v)}
          options={[
            { value: "mica", label: "Mica" },
            { value: "solid", label: "Solid" },
          ]}
        />
      </Row>
    </Section>
  );
};

const PREVIEW_TEXT: StreamingTranscriptPayload = {
  committed_prefix: "",
  mutable_suffix: "",
  full_text: "Inserted",
  language: "en",
  audio_level: 0.2,
  stage: "",
};

const PillSection: React.FC<{ settings: AppSettings; a: A; change: Change }> = ({
  settings,
  a,
  change,
}) => {
  const [phase, setPhase] = useState<AppState>("RECORDING");
  const [level, setLevel] = useState(0.2);
  const listening = phase === "RECORDING";

  // A gentle stand-in voice so the visualizer can be judged. Holds still with reduce motion.
  useEffect(() => {
    if (!listening || a.reduce_motion) return;
    const timer = setInterval(() => setLevel(0.08 + Math.random() * 0.26), 140);
    return () => clearInterval(timer);
  }, [listening, a.reduce_motion]);

  const posIndex = Math.max(
    0,
    POSITIONS.findIndex((p) => p.id === a.overlay_position),
  );
  const onPosKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 3, ArrowUp: -3 }[e.key];
    if (!step) return;
    e.preventDefault();
    const next = (posIndex + step + POSITIONS.length) % POSITIONS.length;
    change("overlay_position", POSITIONS[next].id);
    e.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  };

  return (
    <Section
      icon={<Mic className="w-4 h-4" />}
      title="Dictation pill"
      description={`The capsule that floats over your apps while you dictate with ${settings.hotkey}.`}
    >
      <div>
        <div
          className="pill-stage"
          role="radiogroup"
          aria-label="Pill position on screen"
          tabIndex={-1}
          onKeyDown={onPosKey}
        >
          <span className="pill-stage-window" aria-hidden>
            <i />
            <i />
            <i />
          </span>
          {POSITIONS.map((p, i) => {
            const on = p.id === a.overlay_position;
            return (
              <button
                key={p.id}
                type="button"
                role="radio"
                aria-checked={on}
                aria-label={p.label}
                title={on ? undefined : `Move to ${p.label.toLowerCase()}`}
                tabIndex={i === posIndex ? 0 : -1}
                className="pill-slot"
                data-anchor={p.id}
                onClick={() => change("overlay_position", p.id)}
              >
                {on && (
                  <span aria-hidden>
                    <Overlay
                      appState={phase}
                      transcript={{ ...PREVIEW_TEXT, audio_level: a.reduce_motion ? 0.2 : level }}
                      standalone
                      hudTheme={a.overlay_theme}
                      waveformStyle={a.waveform_style}
                      hudScale={a.hud_scale}
                      hudShape={a.hud_shape}
                      hudStyle={a.hud_style}
                      hudOpacity={a.hud_opacity}
                    />
                  </span>
                )}
              </button>
            );
          })}
        </div>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
          <Choice
            label="Preview state"
            className="segmented-control max-w-full flex-wrap"
            value={phase}
            onChange={setPhase}
            options={PREVIEW_PHASES.map((p) => ({ value: p.id, label: p.label }))}
          />
          <p className="text-xs text-muted">
            <Monitor className="inline w-3.5 h-3.5 mr-1 -mt-0.5" aria-hidden />
            Shown at actual size. Click a spot to move it.
          </p>
        </div>
      </div>

      <Block
        label="Style"
        hint="Waveform shows audio bars while recording and transcribing, then a checkmark when done."
      >
        <Choice
          label="Pill style"
          value={a.hud_style}
          onChange={(v) => change("hud_style", v)}
          options={[
            { value: "status", label: "Status pill" },
            { value: "waveform", label: "Waveform pill" },
          ]}
        />
      </Block>

      <Row label="Size" hint="Scales the pill in both styles.">
        <Choice
          label="Pill size"
          value={a.hud_scale}
          onChange={(v) => change("hud_scale", v)}
          options={[
            { value: "compact", label: "Compact" },
            { value: "standard", label: "Standard" },
            { value: "large", label: "Large" },
          ]}
        />
      </Row>

      <Row label="Shape">
        <Choice
          label="Pill shape"
          value={a.hud_shape}
          onChange={(v) => change("hud_shape", v)}
          options={[
            { value: "pill", label: "Pill" },
            { value: "rounded", label: "Rounded" },
            { value: "square", label: "Square" },
          ]}
        />
      </Row>

      <Row label="Colour" hint="Match app follows your theme above.">
        <Choice
          label="Pill colour"
          value={a.overlay_theme}
          onChange={(v) => change("overlay_theme", v)}
          options={[
            { value: "dark", label: "Dark" },
            { value: "light", label: "Light" },
            { value: "auto", label: "Match app" },
          ]}
        />
      </Row>

      <Row label="Background opacity" hint="Lower lets the app behind show through.">
        <Choice
          label="Background opacity"
          value={a.hud_opacity}
          onChange={(v) => change("hud_opacity", v)}
          options={[0.96, 0.85, 0.75, 0.65].map((v) => ({
            value: v,
            label: `${Math.round(v * 100)}%`,
          }))}
        />
      </Row>

      {a.hud_style === "status" && (
        <Row label="While listening" hint="Minimal hides the word and keeps the dot and level.">
          <Choice
            label="Listening visualizer"
            value={a.waveform_style}
            onChange={(v) => change("waveform_style", v)}
            options={[
              { value: "bars", label: "Level bars" },
              { value: "pulse", label: "Dot only" },
              { value: "minimal", label: "Minimal" },
            ]}
          />
        </Row>
      )}

      <Row label="High contrast" hint="Black pill, white text and a solid edge.">
        <Toggle
          on={a.hud_contrast === "high"}
          onChange={(v) => change("hud_contrast", v ? "high" : "standard")}
          ariaLabel="High contrast pill"
        />
      </Row>
    </Section>
  );
};

const ResetRow: React.FC<{ onReset: () => void }> = ({ onReset }) => {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-wrap items-center justify-end gap-3 pt-1">
      {confirming ? (
        <>
          <p className="text-sm text-muted mr-auto sm:mr-0">
            Reset every appearance setting, including the pill?
          </p>
          <button type="button" className="btn btn-ghost" onClick={() => setConfirming(false)}>
            Keep mine
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => {
              setConfirming(false);
              onReset();
            }}
          >
            Reset appearance
          </button>
        </>
      ) : (
        <button type="button" className="btn btn-ghost" onClick={() => setConfirming(true)}>
          <RotateCcw className="w-3.5 h-3.5" aria-hidden />
          Reset appearance
        </button>
      )}
    </div>
  );
};
