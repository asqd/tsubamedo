/** The app's version from package.json, set by vite.config.ts. */
declare const __APP_VERSION__: string;

interface Window {
  /** Set by src/dev/bridge.ts: calls go to the dev bridge instead of Tauri. */
  __TSUBAMEDO_BRIDGE__?: string;
}
