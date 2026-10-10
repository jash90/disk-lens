/** System, na którym działa aplikacja – rozpoznany z user agenta webview (WKWebView / WebView2 / WebKitGTK). */
export type Os = "macos" | "windows" | "linux";

const ua = navigator.userAgent;
export const OS: Os = ua.includes("Mac") ? "macos" : ua.includes("Windows") ? "windows" : "linux";
export const IS_MAC = OS === "macos";

/** Skrót klawiszowy z właściwym modyfikatorem: „⌘O” na macOS, „Ctrl+O” gdzie indziej. */
export function shortcut(key: string): string {
  return IS_MAC ? `⌘${key}` : `Ctrl+${key === "⌫" ? "Backspace" : key}`;
}

export const REVEAL_LABEL = {
  macos: "Pokaż w Finderze",
  windows: "Pokaż w Eksploratorze",
  linux: "Pokaż w menedżerze plików",
}[OS];
