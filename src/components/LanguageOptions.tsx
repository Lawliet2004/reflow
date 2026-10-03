import { DICTATION_LANGUAGES } from "../languages";

export function LanguageOptions({ includeAuto = true }: { includeAuto?: boolean } = {}) {
  return (
    <>
      {includeAuto && <option value="auto">Auto-detect</option>}
      {DICTATION_LANGUAGES.map(({ code, name }) => (
        <option key={code} value={code}>
          {name}
        </option>
      ))}
    </>
  );
}
