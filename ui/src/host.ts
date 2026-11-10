import { isTauri } from "@tauri-apps/api/core";

/**
 * Whether the page runs in the desktop app, not on a phone through the relay, where what only the
 * computer does is not shown: files saved there, its updates and settings (docs/ui.md).
 */
export const desktop = import.meta.env.MODE !== "phone";

/**
 * docs/ui.md#布局: the height in px of the band at the top of the macOS app's window, where the
 * window's buttons sit over the page, which keeps clear of them and moves the window when dragged;
 * 0 elsewhere.
 */
export const titleBar = isTauri() && navigator.userAgent.includes("Mac") ? 28 : 0;

/**
 * Lets the macOS window be moved by dragging the element, and zoomed by double-clicking it: only
 * where nothing covers it, or with `deep`, anywhere in it but on what can be clicked.
 */
export function dragsWindow(deep?: "deep") {
  return titleBar > 0 ? { "data-tauri-drag-region": deep ?? "" } : {};
}
