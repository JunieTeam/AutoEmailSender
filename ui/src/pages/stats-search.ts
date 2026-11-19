import type { UsagePurpose } from "@/api/types";
import { pageParam } from "@/components/mentor-table";

export const PURPOSES: Record<UsagePurpose, string> = {
  crawl: "智能整理",
  match: "匹配分析",
  draft: "AI 起草",
  rewrite: "AI 改写",
  fact_check: "AI 核对",
  reply_outcome: "回信标记",
  reply_todos: "回信待办",
};

export function isPurpose(p: unknown): p is UsagePurpose {
  return typeof p === "string" && Object.hasOwn(PURPOSES, p);
}

/**
 * The list of model calls on the statistics page (docs/ui.md 统计): whether it is open, and its
 * filters and page. Apart from the page, which loads only when opened, so the router has it at once.
 */
export function statsSearch(s: Record<string, unknown>): {
  calls?: true;
  purpose?: UsagePurpose;
  model?: string;
  callPage?: number;
} {
  return {
    calls: s.calls === true ? true : undefined,
    purpose: isPurpose(s.purpose) ? s.purpose : undefined,
    model: typeof s.model === "string" && s.model !== "" ? s.model : undefined,
    callPage: pageParam(s.callPage),
  };
}
