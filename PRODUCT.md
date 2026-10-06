# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(Tauri 2 desktop shell — WebView2 on Windows, WebKit on macOS/Linux. React 19 + Tailwind v4.)

## Users

People who write a lot on a computer — developers, writers, knowledge workers — and want to speak instead of type, in any app, without sending audio to a cloud service.

## Product Purpose

Reflow is local-first desktop dictation in the style of Wispr Flow. Hold a hotkey anywhere, speak, release; Reflow transcribes on-device, optionally polishes the text with a local LLM, and inserts it into the focused app. Success: the user forgets the tool exists between dictations and trusts what it inserts.

## Positioning

Recognition (Qwen3-ASR / Phonon-2) and cleanup run entirely on the user's machine. Audio never leaves it; airplane mode and a network ledger make that verifiable.

## Operating Context

- Mostly invisible: a global hotkey and a floating recording HUD do the daily work.
- The main window is visited to set up models, review/correct transcripts, take notes, search history, import audio files, and tune settings.
- First run requires downloading a speech model; until then dictation is blocked.

## Capabilities and Constraints

- Surfaces: Home (dictate), Notes, History, Settings (General incl. Output, Modes & Snippets, Appearance, Speech, Performance, Writing, Dictionary, Phone, Advanced), Onboarding wizard, Overlay HUD window, assistant answer card.
- Cleanup levels: Original, Clean, Polished, Refined (the last two need a local LLM).
- Light/dark/system theme, user-selectable accent (sky, indigo, emerald, amber, rose, violet, graphite), UI font scale, reduce motion, HUD contrast.
- Windows 11 Mica behind chrome; custom title bar.
- Browser preview runs with fallback data (no dictation).

## Brand Commitments

- Name: Reflow. Logo at `src/assets/logo.png`.
- Voice: calm, warm, second-person, short ("Speak freely.", "Ready when you are", "One less thought to lose.").
- Visual direction chosen by the user (2026-10-05): warm & editorial — writing-focused, paper-like.

## Evidence on Hand

README feature list and docs/ only. No testimonials, metrics or customer claims exist; do not invent any.

## Product Principles

1. Words first: the transcript is the product; chrome recedes around it.
2. Private by construction: say "on your computer" plainly and never imply otherwise.
3. One obvious next step: especially when no speech model is installed.
4. Calm while live: recording and processing states must read at a glance without alarm.

## Accessibility & Inclusion

Keyboard operable throughout, WCAG AA text contrast, reduced-motion honoured (OS and in-app), high-contrast HUD option, UI font scaling.
