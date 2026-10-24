---
version: 1
slug: "src-components-settings-modelpage-tsx"
primary_target: "src/components/settings/ModelPage.tsx"
related_targets:
  [
    "src/components/SettingsView.tsx",
    "src/components/settings/modelCatalog.tsx",
    "src/components/settings/WritingCatalog.tsx",
    "src/components/settings/PresetSelector.tsx",
  ]
---

# Models and Performance

Mode: Operate. These settings serve people choosing local speech and writing models and diagnosing their hardware. The user delegated design decisions and requested implementation without model downloads. This is a new organization within Reflow's established manuscript-desk visual system, not a replacement identity.

## Direction contract

THESIS: Separate choosing a model from tuning its execution. Model names lead ruled catalog rows; requirements and provenance are available through a disclosure.

OWN-WORLD: Inherit paper surfaces, warm ink, serif section headings, token-based controls and restrained semantic status colors from DESIGN.md. Use shared wells and hairline separators.

STORY: Choose a task, compare model purpose and requirements, inspect its source and runtime status, then explicitly install or select. Unavailable runtimes explain the blocker. Performance presents hardware, compute settings and diagnostics.

FIRST VIEWPORT: Models heading, concise explanation, task filters, current speech state and model list. Performance begins with hardware and presets before detailed runtime controls. Download actions must clearly name their intent and respect airplane mode.

FORM: Task-filtered catalog with progressive disclosure, selected directly under the user's delegated design authority. Code-led extension of the established visual system; no new visual-world seed.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Implemented organization

- Models has four task filters: All models, Speech, Text polish and Meeting notes. Speech starts with the current engine state, followed by ruled model rows. Writing distinguishes text polish selection from the separate meeting-notes model, including the option to follow the text polish model. Downloaded files remain in the On this computer section.
- Each catalog row leads with a model name, purpose, size and installation state. Native Model details disclosures expose purpose, size and memory, language coverage, runtime, license and the source link. Unsupported speech entries retain an explicit runtime blocker and have no install action.
- Speech actions distinguish Use from Download & use and Resume & use. Writing separates selection from Download or Install runtime. Airplane mode disables download actions; deletion uses an inline confirmation. These controls communicate local state without claiming unmeasured model quality or speed.
- Performance begins with Hardware, then Performance preset, Speech engine, Writing processor, Runtime controls, Memory and Diagnostics. Hardware uses processor, memory and graphics facts with honest unavailable states in browser preview. Presets use ruled radio rows and a current hardware-plan summary; the selected row is accented, and arrow keys change selection.

## Design evidence

The implementation preserves the manuscript desk: warm paper sheets and wells, serif page and section headings, quiet sans-serif controls, hairline row separators, and existing theme tokens. Catalog rows and presets share a container rather than becoming separate cards. Details remain subordinate until expanded. No new palette, global component system or imagery was introduced; DESIGN.md and .impeccable/design.json remain unchanged.

Browser-preview captures in `.impeccable/review/` record two viewport classes (1100 × 800 desktop and 640 × 640 compact): `desktop.png` and `compact.png` show Models; `notes-desktop.png` and `notes-compact.png` show the Meeting notes filter; `performance-desktop.png` and `performance-compact.png` show Performance. Desktop retains the Settings rail; compact uses the existing stacked Settings layout and scrollable content. The captures show browser fallback data with airplane mode enabled, not installed-model or live-inference evidence. They are review evidence, not shipping raster assets.

The finish review's ordering finding was addressed by moving Performance preset directly after Hardware. Both Performance captures show the corrected order. Final independent review verdict: **ship**, scoped to the reviewed screenshots and layout. The build passed after the ordering adjustment. Native inference remains unverified; no model downloads were performed for this review.

Runtime-first follow-up: model actions now communicate runtime prerequisites and the sidebar preserves setup status outside Settings. Existing catalog typography and ruled rows are retained. Speech orchestration survives Settings closure and refuses weight download if settings change. Verification uses mocked setup lifecycles; native live installation remains deliberately untested to honor the no-download request.
