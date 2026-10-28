// An open request in the window.
import { Request, ResponseMeta } from "./api";
import { t } from "./i18n";

export interface Tab {
  key: string;
  /** The collection entry, or null for a scratch request that lives only in the window. */
  id: string | null;
  request: Request;
  response: ResponseMeta | null;
  /** The environment the response came from. */
  responseEnv?: string | null;
  error: string | null;
  sendId: string | null;
  /** Opened and not touched: the next request picked in the sidebar replaces it. */
  pristine: boolean;
}

export function tabTitle(tab: Tab): string {
  if (tab.request.name) return tab.request.name;
  if (tab.id) return tab.id.split("/").pop()!;
  return tab.request.url.replace(/^https?:\/\//, "").split("?")[0] || t("Черновик", "Draft");
}
