// The window's language: Russian or English. Every text is written in both, side by side, so
// neither drifts from the other: t("Отправить", "Send"). The language is settled before the
// window is drawn and changes with a reload, so components read it plainly.
import { api } from "./api";

export type Lang = "ru" | "en";
/** What the user chose: the system's language, or one of them. */
export type LangChoice = "auto" | Lang;

let current: Lang = "ru";

export const lang = () => current;

export const t = (ru: string, en: string) => (current === "en" ? en : ru);

/** Russian when the system's first language is; English for everyone else. */
export function systemLang(): Lang {
  return (navigator.languages?.[0] ?? navigator.language ?? "en").toLowerCase().startsWith("ru") ? "ru" : "en";
}

export const resolve = (choice: LangChoice): Lang => (choice === "auto" ? systemLang() : choice);

/** The chosen language, before the window is drawn; the core speaks it too from then on. */
export async function loadLang(): Promise<LangChoice> {
  let choice: LangChoice = "auto";
  try {
    choice = (await api.language()).language;
  } catch {
    // Without the core the window still opens, in the system's language.
  }
  current = resolve(choice);
  document.documentElement.lang = current;
  api.setLanguage(current, null).catch(() => {});
  return choice;
}

/** Keeps the choice and redraws the window in it; open tabs come back as they were. */
export async function chooseLang(choice: LangChoice) {
  await api.setLanguage(resolve(choice), choice);
  window.location.reload();
}
