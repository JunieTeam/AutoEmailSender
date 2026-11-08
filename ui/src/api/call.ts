import { invoke } from "@tauri-apps/api/core";
import type { CallError, ErrorCode, Failure, Operations } from "./types";

/** Runs an operation in the core (crates/api). Rejects with a `CallError`. */
export function call<K extends keyof Operations>(
  op: K,
  input: Operations[K]["input"],
): Promise<Operations[K]["output"]> {
  return invoke<Operations[K]["output"]>("call", { op, input });
}

/** What the UI says of a defect of its own, which it logs (docs/development.md#错误和日志). */
export const UI_DEFECT = "界面出错，请重试；仍然出错的，可在电脑上的 设置 › 关于和日志 里反馈问题";

/** A failure the UI finds itself, such as a file too large to add, shown like the core's. */
export class UiError extends Error implements CallError {
  code: ErrorCode = "invalid";
  details: string | null;
  constructor(message: string, details: string | null = null) {
    super(message);
    this.details = details;
  }
}

/** Why something failed, for anything thrown: what a failed call says, or else a defect. */
export function failureOf(error: unknown): Failure {
  return isCallError(error)
    ? { message: error.message, details: error.details }
    : { message: UI_DEFECT, details: null };
}

/** The message alone, for places that only take text, such as a notification. */
export function errorMessage(error: unknown): string {
  return failureOf(error).message;
}

/** The code of a failed call, for the few failures a page answers itself. */
export function errorCode(error: unknown): ErrorCode | undefined {
  return isCallError(error) ? error.code : undefined;
}

const CODES: Record<ErrorCode, true> = {
  bad_input: true,
  not_found: true,
  invalid: true,
  external: true,
  signed_out: true,
  needs_agreement: true,
  account_deleting: true,
  internal: true,
};

export function isCallError(error: unknown): error is CallError {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof error.code === "string" &&
    Object.hasOwn(CODES, error.code) &&
    "message" in error &&
    typeof error.message === "string" &&
    "details" in error &&
    (error.details === null || typeof error.details === "string")
  );
}
