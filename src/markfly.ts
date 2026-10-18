import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A document tab: a local markdown file or an imported remote URL. */
export interface TabItem {
  path: string;
  name: string;
  active?: boolean;
}

export type UpdateStatus =
  | "idle"
  | "checking"
  | "current"
  | "available"
  | "downloading"
  | "ready"
  | "opening"
  | "error";

export interface UpdateState {
  revision: number;
  status: UpdateStatus;
  currentVersion: string;
  latestVersion: string | null;
  releaseInfoAvailable: boolean;
  bytesReceived: number;
  totalBytes: number | null;
  error: string | null;
}

export interface FetchedMarkdown {
  content: string;
  url: string;
}

export interface Heading {
  id: string;
  level: number;
  text: string;
}

/**
 * Typed bridge to the Rust command handlers. This replaces the Electron
 * `window.electronAPI` bridge: every method maps 1:1 to a `#[tauri::command]`
 * in `src-tauri/src/commands.rs`.
 */
export const markfly = {
  readFile: (path: string) => invoke<string | null>("read_file", { path }),
  copyText: (text: string) => invoke<boolean>("copy_text", { text }),
  fetchMarkdownUrl: (url: string) =>
    invoke<FetchedMarkdown>("fetch_markdown_url", { url }),
  watchFile: (path: string) => invoke<void>("watch_file", { path }),
  getArgs: () => invoke<string | null>("get_args"),
  rendererReady: () => invoke<void>("renderer_ready"),
  openExternal: (url: string) => invoke<boolean>("open_external", { url }),
  getUpdateState: () => invoke<UpdateState>("get_update_state"),
  checkForUpdates: () => invoke<UpdateState>("update_check"),
  downloadUpdate: () => invoke<UpdateState>("update_download"),
  openUpdateInstaller: () => invoke<UpdateState>("update_open_installer"),
  openUpdateRelease: () => invoke<void>("update_open_release"),
  onUpdateState: (cb: (state: UpdateState) => void): Promise<UnlistenFn> =>
    listen<UpdateState>("markfly://update-state", (event) => cb(event.payload)),
  getTheme: () => invoke<"light" | "dark">("get_theme"),
  openFileDialog: () => invoke<string | null>("open_file_dialog"),
  openFilePath: (path: string) => invoke<boolean>("open_file_path", { path }),
  getRecentFiles: () => invoke<string[]>("get_recent_files"),
  onFileChanged: (cb: () => void): Promise<UnlistenFn> =>
    listen("markfly://file-changed", () => cb()),
  onOpenFile: (cb: (path: string) => void): Promise<UnlistenFn> =>
    listen<string>("markfly://open-file", (event) => cb(event.payload)),
  loadPref: () => invoke<Record<string, unknown>>("load_pref"),
  savePref: (obj: Record<string, unknown>) => invoke<void>("save_pref", { obj }),
  renderPreview: (markdown: string) => invoke<string>("render_preview", { markdown }),
  getOutline: (markdown: string) => invoke<Heading[]>("get_outline", { markdown }),
};

/** `~`-shortened display path for the sidebar (macOS and Windows home dirs). */
export function displayPath(path: string): string {
  if (path.startsWith("https://") || path.startsWith("http://")) return path;
  const macos = path.startsWith("/Users/")
    ? (() => {
        const after = path.slice("/Users/".length);
        const index = after.indexOf("/");
        return index >= 0 ? `~${after.slice(index)}` : null;
      })()
    : null;
  if (macos) return macos;
  const windows = /^[A-Za-z]:\\Users\\([^\\]+)\\(.*)$/.exec(path);
  if (windows) return `~\\${windows[2]}`;
  return path;
}
