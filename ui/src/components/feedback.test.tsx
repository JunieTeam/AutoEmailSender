// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CallError, FeedbackInput } from "@/api/types";
import { FeedbackDialog } from "./feedback";

const remote = vi.hoisted(() => {
  const submitted: FeedbackInput[] = [];
  return { submitted, email: "lisi@163.com", fail: true, answered: Promise.resolve(), previews: 0 };
});
vi.mock("@/api/call", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/call")>()),
  call: async (op: string, input: FeedbackInput) => {
    if (op === "getAccount") return { kind: "loggedIn", email: remote.email, termsOutdated: false, cloudEmails: 0 };
    if (op === "previewFeedback") {
      remote.previews += 1;
      return { size: 88_064, days: { first: 0, last: 0 } };
    }
    if (op === "getFeedbackLog") return `log ${remote.previews}`;
    if (op === "submitFeedback") {
      remote.submitted.push(input);
      await remote.answered;
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

async function write(text: string) {
  const box = document.querySelector("textarea");
  if (!box) throw new Error("missing description");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function shownLog() {
  return document.querySelector("pre")?.textContent;
}

function button(text: string) {
  return Array.from(document.querySelectorAll("button")).find((item) => item.textContent === text);
}

afterEach(() => {
  document.body.innerHTML = "";
  Object.assign(remote, { email: "lisi@163.com", fail: true, answered: Promise.resolve() });
});

function render() {
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
  const close = async () => {
    await act(async () => root.unmount());
    client.clear();
  };
  return { client, root, open, close };
}

it("keeps what was written through closing and a failed try, and shows the log the next one sends", async () => {
  const { root, open, close } = render();
  try {
    await open();
    await vi.waitFor(() => expect(document.body.textContent).toContain("运行日志（86 KB）"));
    expect(document.body.textContent).toContain("回复会发到你的账号邮箱 l***@163.com");
    expect(button("提交")?.disabled).toBe(true);
    await write("导出日志时卡住了");
    await act(async () => root.render(<></>));
    await open();
    await vi.waitFor(() => expect(written()).toBe("导出日志时卡住了"));
    await act(async () => button("查看")?.click());
    await vi.waitFor(() => expect(shownLog()).toBe(`log ${remote.previews}`));

    await act(async () => button("提交")?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("反馈次数已达上限，请在 5 小时后再试"));
    expect(button("导出日志")).toBeDefined();
    expect(written()).toBe("导出日志时卡住了");

    remote.fail = false;
    const before = remote.previews;
    await act(async () => button("提交")?.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("已发给开发者"));
    expect(written()).toBe("");
    // A new snapshot is taken for the next feedback, and its log shown in place of the one sent.
    await vi.waitFor(() => expect(shownLog()).toBe(`log ${before + 1}`));
    expect(remote.submitted).toEqual([
      { description: "导出日志时卡住了", withLog: true },
      { description: "导出日志时卡住了", withLog: true },
    ]);
  } finally {
    await close();
  }
});

it("says the reply goes to the account that sent the feedback, though another has logged in since", async () => {
  const { client, open, close } = render();
  remote.fail = false;
  let answer: (() => void) | undefined;
  remote.answered = new Promise((resolve) => (answer = resolve));
  try {
    await open();
    await vi.waitFor(() => expect(document.body.textContent).toContain("运行日志"));
    await write("收不到回信");
    await act(async () => button("提交")?.click());
    await vi.waitFor(() => expect(button("正在提交…")).toBeDefined());
    remote.email = "wangwu@qq.com";
    await act(() => client.invalidateQueries({ queryKey: ["getAccount"] }));
    await vi.waitFor(() => expect(document.body.textContent).toContain("回复会发到你的账号邮箱 w***@qq.com"));
    await act(async () => answer?.());
    await vi.waitFor(() => expect(document.body.textContent).toContain("已发给开发者，回复会发到 l***@163.com"));
  } finally {
    await close();
  }
});
