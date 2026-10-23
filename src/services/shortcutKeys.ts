/** Modifier-only desktop shortcuts fire on release on KDE, not while held. */
export function isModifierOnlyShortcut(shortcut: string | null | undefined): boolean {
  const aliases: Record<string, string> = {
    ctrl: "ctrl",
    control: "ctrl",
    shift: "shift",
    alt: "alt",
    win: "meta",
    super: "meta",
    meta: "meta",
    cmd: "meta",
  };
  const keys = (shortcut ?? "").split("+").map((key) => aliases[key.trim().toLowerCase()]);
  return keys.every(Boolean) && new Set(keys).size >= 2;
}
