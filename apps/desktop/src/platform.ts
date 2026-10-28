// Native parts: dialogs in the app, plain prompts in the dev bridge.
import { open, save } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";

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
  if (inBridge) return prompt(t("Папка коллекции (в мосте путь вводится вручную)", "Collection folder (type the path by hand in the bridge)"));
  const picked = await open({ directory: true, title: t("Папка коллекции", "Collection folder") });
  return typeof picked === "string" ? picked : null;
}

/** A file to import: exports of Insomnia and Postman, OpenAPI, HAR. */
export async function pickImportFile(): Promise<string | null> {
  if (inBridge) return prompt(t("Файл для импорта (в мосте путь вводится вручную)", "File to import (type the path by hand in the bridge)"));
  const picked = await open({
    title: t("Импорт: Insomnia, Postman, OpenAPI или HAR", "Import: Insomnia, Postman, OpenAPI or HAR"),
    filters: [{ name: t("Коллекции и описания API", "Collections and API descriptions"), extensions: ["json", "yaml", "yml", "har"] }],
  });
  return typeof picked === "string" ? picked : null;
}

/** A folder to create a new collection in. */
export async function pickParentFolder(): Promise<string | null> {
  if (inBridge) return prompt(t("Папка, в которой создать коллекцию (в мосте путь вводится вручную)", "Folder to create the collection in (type the path by hand in the bridge)"));
  const picked = await open({ directory: true, title: t("Где создать коллекцию", "Where to create the collection") });
  return typeof picked === "string" ? picked : null;
}

/** A client certificate or its key. */
export async function pickCertFile(title: string): Promise<string | null> {
  if (inBridge) return prompt(t(`${title} (в мосте путь вводится вручную)`, `${title} (type the path by hand in the bridge)`));
  const picked = await open({
    title,
    filters: [{ name: t("Сертификаты и ключи", "Certificates and keys"), extensions: ["pem", "crt", "cer", "key", "p12", "pfx"] }],
  });
  return typeof picked === "string" ? picked : null;
}

export async function pickSavePath(name: string): Promise<string | null> {
  if (inBridge) return prompt(t("Куда сохранить", "Save to"), `/tmp/${name}`);
  return save({ defaultPath: name, title: t("Сохранить ответ", "Save response") });
}

/** The home folder as "~". */
export function home(p: string): string {
  return p.replace(/^(\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:[\\/]Users[\\/][^\\/]+)/, "~");
}
