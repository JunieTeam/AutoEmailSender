import { call, isCallError } from "./api/call";

/** When each message was last logged: one that keeps coming is logged once a minute. */
const logged = new Map<string, number>();

/**
 * Writes a defect of the UI's own to the log (docs/development.md#错误和日志). A failure to write it
 * is let go: reporting that would come back here.
 */
export function logUiError(error: unknown) {
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  const now = Date.now();
  if (now - (logged.get(message) ?? -Infinity) < 60_000) return;
  logged.set(message, now);
  const stack = error instanceof Error ? (error.stack ?? null) : null;
  // Loaded by now; not imported up front, as the router's pages log through this module.
  void import("./router")
    .then(({ router }) => call("logUiError", { message, stack, route: router.state.matches.at(-1)?.routeId ?? "" }))
    .catch(() => undefined);
}

/** Logs what was thrown unless it is a failure the core or the UI already explains. */
export function logUnexplained(error: unknown) {
  if (!isCallError(error)) logUiError(error);
}

/** Logs errors nothing else caught, once, at the page's start. */
export function listenForUiErrors() {
  window.addEventListener("error", (e) => logUiError(e.error ?? e.message));
  window.addEventListener("unhandledrejection", (e) => logUnexplained(e.reason));
}
