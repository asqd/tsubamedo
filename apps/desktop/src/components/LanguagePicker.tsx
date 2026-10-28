// The language of «Код…»: the list grouped as the core groups the languages (Shell, Python,
// JavaScript…), each by its library under its group.
import { Language } from "../api";
import { t } from "../i18n";
import { Picker } from "./Picker";

/** «Python — requests»; a group of one name, and the shell, by the name alone. */
export const languageLabel = (l: Language) => (l.group === l.name || l.group === "Shell" ? l.name : `${l.group} — ${l.name}`);

export function LanguagePicker({ languages, value, onChange }: { languages: Language[]; value: string; onChange: (id: string) => void }) {
  return (
    <Picker
      className="language-picker"
      label={t("Язык", "Language")}
      title={t("Язык и библиотека", "Language and library")}
      options={languages.map((l) => ({ value: l.id, label: languageLabel(l), item: l.name, group: l.group }))}
      value={value}
      onChange={onChange}
    />
  );
}
