import { expect, it } from "vitest";
import { isModifierOnlyShortcut } from "./shortcutKeys";

it.each(["Ctrl+Shift", "Control+Super", "Shift+Meta", "Ctrl+Shift+Win"])(
  "recognizes %s as a KDE tap shortcut",
  (key) => expect(isModifierOnlyShortcut(key)).toBe(true),
);
it.each(["Ctrl+Space", "Shift+Space", "Ctrl+1", "Ctrl+K", "Ctrl+Ctrl", "", null])(
  "does not override hold-to-talk for %s",
  (key) => expect(isModifierOnlyShortcut(key)).toBe(false),
);
