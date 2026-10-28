// Numbers, sizes and dates in the window's language: "12,3 КБ", "1,23 с" in Russian, "12.3 KB",
// "1.23 s" in English.
import { lang, t } from "./i18n";

const locale = () => (lang() === "en" ? "en-US" : "ru-RU");
const point = (s: string) => (lang() === "en" ? s : s.replace(".", ","));

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} ${t("Б", "B")}`;
  const units = lang() === "en" ? ["KB", "MB", "GB"] : ["КБ", "МБ", "ГБ"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${point(value < 10 ? value.toFixed(1) : value.toFixed(0))} ${units[unit]}`;
}

export function ms(value: number): string {
  if (value < 1) return `${point(value.toFixed(1))} ${t("мс", "ms")}`;
  if (value < 1000) return `${Math.round(value)} ${t("мс", "ms")}`;
  return `${point((value / 1000).toFixed(2))} ${t("с", "s")}`;
}

export function count(n: number): string {
  return n.toLocaleString(locale());
}

/** 1 строка, 2 строки, 5 строк. */
export function plural(n: number, one: string, few: string, many: string): string {
  const d = n % 10;
  const h = n % 100;
  if (d === 1 && h !== 11) return one;
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return few;
  return many;
}

/** The word for `n` in the window's language: noun(5, ["строка", "строки", "строк"], ["line", "lines"]). */
export function noun(n: number, ru: [string, string, string], en: [string, string]): string {
  return lang() === "en" ? (n === 1 ? en[0] : en[1]) : plural(n, ...ru);
}

export function statusTone(status: number): "ok" | "redirect" | "warn" | "bad" {
  if (status < 300) return "ok";
  if (status < 400) return "redirect";
  if (status < 500) return "warn";
  return "bad";
}

/** «сегодня 14:03», «вчера 09:12», «27 октября 18:40»; "today 14:03", "October 27 18:40". */
export function when(at: number): string {
  const d = new Date(at);
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(today) - start(d)) / 86_400_000);
  const time = d.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", hour12: false });
  const date = days === 0 ? t("сегодня", "today") : days === 1 ? t("вчера", "yesterday") : d.toLocaleDateString(locale(), { day: "numeric", month: "long" });
  return `${date} ${time}`;
}
