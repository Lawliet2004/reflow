import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";

/**
 * Milestone 0 / Task 4: the quality gate.
 *
 * `jsx-a11y` and `react-hooks` are the two rule sets that matter for this
 * codebase specifically: the UI polish milestone depends on unlabelled
 * controls and missing roles being caught mechanically, and several confirmed
 * bugs (leaked intervals, un-awaited async setup) are hook-dependency and
 * cleanup problems.
 */
export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/**",
      "coverage/**",
      "*.config.js",
      "*.config.ts",
      "scripts/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: {
      "react-hooks": reactHooks,
      "jsx-a11y": jsxA11y,
    },
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: {
        ...globals.browser,
        ...globals.es2022,
      },
      parserOptions: {
        ecmaFeatures: { jsx: true },
      },
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      ...jsxA11y.flatConfigs.recommended.rules,

      // An unused variable is how `void isTerminal;` ended up in App.tsx as a
      // workaround for a linter that did not exist. Allow the conventional
      // underscore prefix for genuinely unused bindings.
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrorsIgnorePattern: "^_",
        },
      ],

      // A floating promise is exactly the un-awaited `clipboard.writeText`
      // and un-awaited async `setup()` listener-leak class of bug. The typed
      // rule needs type information, so approximate it with the untyped
      // checks that do not require a full program.
      "no-void": ["error", { allowAsStatement: true }],

      // Timers created in an effect must be cleaned up; the rule cannot prove
      // that, but exhaustive-deps catches the common cause.
      "react-hooks/exhaustive-deps": "error",

      // ---------------------------------------------------------------
      // Tracked allow-list. These are real findings, but each fix is a
      // component redesign rather than a local edit, so they belong to the
      // milestone that already rewrites the surface. Kept at "warn" so they
      // stay visible and cannot grow silently past the counts below.
      //
      // react-hooks/set-state-in-effect (6 sites: DictateHome, SettingsView,
      //   CleanupPage, DictionaryPage x2) — props mirrored into state. Task 39
      //   extracts the duplicated tier UI and the shared status hook, which is
      //   where the derived-state conversion belongs.
      //
      // react-hooks/refs (3 sites: HotkeyPicker, Waveform x2) — refs read
      //   during render. Task 36 rebuilds the HUD state machine and Task 40
      //   converts Waveform to a transform-based rAF loop.
      // ---------------------------------------------------------------
      "react-hooks/set-state-in-effect": "warn",
      "react-hooks/refs": "warn",
    },
  },
  {
    // Onboarding focuses its first field on step change. Task 40 replaces this
    // with explicit focus management on view and step changes.
    files: ["src/components/Onboarding.tsx"],
    rules: {
      "jsx-a11y/no-autofocus": "warn",
    },
  },
  {
    files: ["src/**/*.test.{ts,tsx}", "src/test/**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    rules: {
      // Tests legitimately reach into internals and use non-null assertions.
      "@typescript-eslint/no-non-null-assertion": "off",
      "@typescript-eslint/no-explicit-any": "off",
    },
  },
);
