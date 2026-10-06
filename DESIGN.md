# Design

World: **the manuscript desk** — warm & editorial. Paper sheets on a desk ground, words set in a book face, ink for actions. Mode: Operate (the task and state always outrank expression).

## Tokens (`src/styles/globals.css`)

- **Ground / sheets:** `base` is the desk, `surface` is a sheet. Light = warm paper (`#f4efe6` / `#fbf8f2`); dark = lamp-lit charcoal (`#1a1714` / `#25211c`).
- **Ink:** `ink`, `ink-2`, `muted`, `faint` — four warm steps, all ≥ 4.5:1 on every surface.
- **Accent:** an ink colour, not a highlighter. Seven user choices (`data-accent`), each retinted for paper and for charcoal. Used for focus, selection, caret, links, active icons, the live record ring.
- **Action:** primary buttons are `ink` on paper (cream on charcoal in dark). Disabled primary drops to `surface-3` + `faint`.
- **Type:** `--font-display` / `--font-serif` = installed book faces (Iowan Old Style → Sitka → Charter → Cambria → Georgia), offline by design. Display for page/section headings at weight 500; serif for transcripts and notes. UI chrome stays in Segoe UI Variable / system sans.
- **Elevation:** declared once. Cards use a 1px `line` border, no shadow. Only the Settings dialog, menus and toasts cast warm offset shadows. Dark mode swaps shadows for borders.
- **Well:** `--color-well` (base mixed into surface) is the sunken tint for grouped rows and the Settings rail. Rows inside a well are ruled apart by `line-soft` hairlines, never boxed individually. Secondary buttons are soft fills (`surface-3`), not outlines.
- **Radii:** cards `--radius-card` (14px), controls `--radius-control` (8px), pills only for chips. Never hard-code a radius; the corner setting retunes these.

## User appearance settings

Settings › Appearance retunes tokens through root attributes set by `applyTheme` (and pre-paint by index.html). Components read tokens only, so new UI inherits every option for free:

- `data-tone` warm | neutral | cool — swaps the surface/ink set (each keeps ≥ 4.5:1).
- `data-accent` presets or `custom` (`--accent-custom`; hover/soft/border derived via `color-mix`).
- `data-reading-font` / `data-heading-font` — override `--font-serif` / `--font-display`. `--face-*` stay fixed for pickers.
- `data-font-scale` (root px), `data-density` (`--density` multiplier + Tailwind `--spacing`), `data-corners`.
- HUD: `.hud-scale-*` sets `--hud-w/--hud-h` (must match `overlay_dims` in `overlay.rs`), `data-shape`, `--hud-alpha`.
- When adding shell padding, multiply by `var(--density)`.

## Shell

- **Desk + canvas.** Title bar and sidebar sit directly on `base` with no borders or tint (transparent over Mica). Every page renders inside `.app-canvas`: one `surface` sheet, 1px `line` border, radius `--radius-card + 4px`, inset 8px from the right and bottom.
- **Sidebar.** Brand lockup (logo + "Reflow", sans semibold) at the top; Home / Notes / History; then a contextual card slot (model download progress, or the speech-model setup callout off Home), a hairline, and Settings / Open logs / Quit. Nav is monochrome: active and hover share a flat `base-2` fill, and icons are 18px at 1.75 stroke. The title-bar toggle collapses it to a 64px icon rail (`reflow.sidebar-collapsed` in localStorage).
- **Settings is a modal.** A native `<dialog>` over the current page: `well` category rail with sentence-case group labels (Dictation / App), content on `surface`, sections titled on the sheet with their rows in a well. Esc, the close button or a backdrop click dismiss it. Toasts render inside the dialog while it is open (top layer).

## Signature

The Home **manuscript**: one sheet holding the record seal, status, and the transcript set in serif on a ~44rem measure (`--gutter`). Writing preferences sit under it as a ruled strip, not a card; recent dictations and file import share a secondary row.

## Rules

- No eyebrow labels, no gradient text, no decorative glass.
- Stage colours (`--color-stage-*`) are information; keep them distinct.
- Respect `.reduce-motion` and `prefers-reduced-motion`; the record ring holds still.
