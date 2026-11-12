import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app";
import "./index.css";
import { titleBar } from "./host";
import { followTheme } from "./theme";
import { listenForUiErrors } from "./ui-errors";

// Before the first render, so the page never shows in the other appearance or under the window's
// buttons.
followTheme();
document.documentElement.style.setProperty("--title-bar", `${titleBar}px`);
listenForUiErrors();
// docs/ui.md#交互: the WebView's own menu (Reload, Open Link, Inspect Element) is not the app's. It
// opens only on what is typed in and on selected text, to copy and paste.
document.addEventListener("contextmenu", (e) => {
  const typed = e.target instanceof Element && e.target.closest("input, textarea, [contenteditable='true']");
  if (!typed && (getSelection()?.isCollapsed ?? true)) e.preventDefault();
});

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
