// Native parts: dialogs in the app, plain prompts in the dev bridge.
import { open, save } from "@tauri-apps/plugin-dialog";

export const isMac = /Mac/.test(navigator.userAgent);
export const inBridge = Boolean(window.__TSUBAMEDO_BRIDGE__);

/** A shortcut as the system names it: "⌘↵" on a Mac, "Ctrl+↵" elsewhere. */
export function key(k: string): string {
  return isMac ? `⌘${k}` : `Ctrl+${k}`;
}

/** ⌘ on a Mac, Ctrl elsewhere. */
export function mod(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

export async function pickFolder(): Promise<string | null> {
  if (inBridge) return prompt("Папка коллекции (в мосте путь вводится вручную)");
  const picked = await open({ directory: true, title: "Папка коллекции" });
  return typeof picked === "string" ? picked : null;
}

export async function pickSavePath(name: string): Promise<string | null> {
  if (inBridge) return prompt("Куда сохранить", `/tmp/${name}`);
  return save({ defaultPath: name, title: "Сохранить ответ" });
}

/** The home folder as "~". */
export function home(p: string): string {
  return p.replace(/^(\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:[\\/]Users[\\/][^\\/]+)/, "~");
}
