import { dictationLanguagesForModel } from "../languages";

export function LanguageOptions({
  includeAuto = true,
  model,
}: { includeAuto?: boolean; model?: string } = {}) {
  return (
    <>
      {includeAuto && <option value="auto">Auto-detect</option>}
      {dictationLanguagesForModel(model).map(({ code, name }) => (
        <option key={code} value={code}>
          {name}
        </option>
      ))}
    </>
  );
}
