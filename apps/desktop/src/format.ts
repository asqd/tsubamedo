// Numbers the Russian way: "12,3 КБ", "1,23 с", "28 000 000".

const comma = (s: string) => s.replace(".", ",");

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`;
  const units = ["КБ", "МБ", "ГБ"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${comma(value < 10 ? value.toFixed(1) : value.toFixed(0))} ${units[unit]}`;
}

export function ms(value: number): string {
  if (value < 1) return `${comma(value.toFixed(1))} мс`;
  if (value < 1000) return `${Math.round(value)} мс`;
  return `${comma((value / 1000).toFixed(2))} с`;
}

export function count(n: number): string {
  return n.toLocaleString("ru-RU");
}

/** 1 строка, 2 строки, 5 строк. */
export function plural(n: number, one: string, few: string, many: string): string {
  const d = n % 10;
  const h = n % 100;
  if (d === 1 && h !== 11) return one;
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return few;
  return many;
}

export function statusTone(status: number): "ok" | "redirect" | "warn" | "bad" {
  if (status < 300) return "ok";
  if (status < 400) return "redirect";
  if (status < 500) return "warn";
  return "bad";
}

/** «сегодня 14:03», «вчера 09:12», «27 октября 18:40». */
export function when(at: number): string {
  const d = new Date(at);
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(today) - start(d)) / 86_400_000);
  const time = d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  const date = days === 0 ? "сегодня" : days === 1 ? "вчера" : d.toLocaleDateString("ru-RU", { day: "numeric", month: "long" });
  return `${date} ${time}`;
}
