// An open request in the window.
import { Request, ResponseMeta } from "./api";

export interface Tab {
  key: string;
  /** The collection entry, or null for a scratch request that lives only in the window. */
  id: string | null;
  request: Request;
  response: ResponseMeta | null;
  error: string | null;
  sendId: string | null;
  /** Opened and not touched: the next request picked in the sidebar replaces it. */
  pristine: boolean;
}

export function tabTitle(tab: Tab): string {
  if (tab.request.name) return tab.request.name;
  if (tab.id) return tab.id.split("/").pop()!;
  return tab.request.url.replace(/^https?:\/\//, "").split("?")[0] || "Черновик";
}
