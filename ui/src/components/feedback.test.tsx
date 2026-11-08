// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CallError, FeedbackInput } from "@/api/types";
import { FeedbackDialog } from "./feedback";

const remote = vi.hoisted(() => {
  const submitted: FeedbackInput[] = [];
  return { submitted, fail: true };
});
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: FeedbackInput) => {
    if (op === "getAccount") return { kind: "loggedIn", email: "lisi@163.com", termsOutdated: false, cloudEmails: 0 };
    if (op === "previewFeedback") return { size: 88_064, days: { first: 0, last: 0 } };
    if (op === "submitFeedback") {
      remote.submitted.push(input);
      if (remote.fail)
        throw { code: "external", message: "反馈次数已达上限，请在 5 小时后再试", details: null } satisfies CallError;
      return null;
    }
    throw new Error(`unexpected operation: ${op}`);
  },
}));

function written() {
  return document.querySelector("textarea")?.value;
}

function button(text: string) {
  return Array.from(document.querySelectorAll("button")).find((item) => item.textContent === text);
}

afterEach(() => {
  document.body.innerHTML = "";
});

it("keeps what was written through closing and a failed try, and clears it once sent", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  const open = () =>
    act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <FeedbackDialog onClose={() => {}} />
        </QueryClientProvider>,
      ),
    );
  try {
    await open();
    await vi.waitFor(() => expect(document.body.textContent).toContain("运行日志（86 KB）"));
    expect(document.body.textContent).toContain("回复会发到你的账号邮箱 l***@163.com");
    expect(button("提交")?.disabled).toBe(true);
    const box = document.querySelector("textarea");
    if (!box) throw new Error("missing description");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(box, "导出日志时卡住了");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => root.render(<></>));
    await open();
    await vi.waitFor(() => expect(written()).toBe("导出日志时卡住了"));

    await act(async () => button("提交")?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("反馈次数已达上限，请在 5 小时后再试"));
    expect(button("导出日志")).toBeDefined();
    expect(written()).toBe("导出日志时卡住了");

    remote.fail = false;
    await act(async () => button("提交")?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("已发给开发者"));
    expect(written()).toBe("");
    expect(remote.submitted).toEqual([
      { description: "导出日志时卡住了", withLog: true },
      { description: "导出日志时卡住了", withLog: true },
    ]);
  } finally {
    await act(async () => root.unmount());
    client.clear();
  }
});
