/** The system the app runs on, recognised from the webview user agent (WKWebView / WebView2 / WebKitGTK). */
type Os = "macos" | "windows" | "linux";

const ua = navigator.userAgent;
export const OS: Os = ua.includes("Mac") ? "macos" : ua.includes("Windows") ? "windows" : "linux";
export const IS_MAC = OS === "macos";

/** Keyboard shortcut with the right modifier: "⌘O" on macOS, "Ctrl+O" elsewhere. */
export function shortcut(key: string): string {
  return IS_MAC ? `⌘${key}` : `Ctrl+${key === "⌫" ? "Backspace" : key}`;
}

export const REVEAL_LABEL = {
  macos: "Pokaż w Finderze",
  windows: "Pokaż w Eksploratorze",
  linux: "Pokaż w menedżerze plików",
}[OS];
