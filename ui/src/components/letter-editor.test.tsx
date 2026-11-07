// @vitest-environment happy-dom
import type { TiptapEditorHTMLElement } from "@tiptap/react";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { LetterEditor, pastedMarkdown } from "./letter-editor";

// Asks for the focus at the user's word, and stops asking once told it is taken.
function Asking() {
  const [focus, setFocus] = useState(false);
  const [version, setVersion] = useState(0);
  return (
    <>
      <button type="button" onClick={() => setFocus(true)}>
        聚焦
      </button>
      <button type="button" onClick={() => setVersion(version + 1)}>
        重建
      </button>
      <LetterEditor
        key={version}
        id="letter"
        markdown="王老师："
        placeholders={[]}
        focus={focus}
        onFocused={() => setFocus(false)}
        disabled={false}
        onChange={() => {}}
      />
    </>
  );
}

it.each([
  ["【请**补充**：面谈时间】", "【请补充：面谈时间】"],
  ["【**请补充：**面谈时间】", "【请补充：面谈时间】"],
  ["【请[补充](https://example.edu)：面谈时间】", "【请补充：面谈时间】"],
  ["【请补充:面谈时间】", "【请补充:面谈时间】"],
  ["【请补\n充：面谈时间】", "【请补充：面谈时间】"],
])("highlights the whole visible marker across inline nodes: %s", async (markdown, visible) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <LetterEditor
          id="letter"
          markdown={`王老师：【重点】时间为${markdown}。`}
          placeholders={[]}
          unfilled
          disabled={false}
          onChange={() => {}}
        />,
      ),
    );
    const highlighted = Array.from(container.querySelectorAll(".tiptap span"), (node) => node.textContent).join("");
    expect(highlighted).toBe(visible);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

it.each([
  ["我有**3 篇论文**", ["我有3 篇论文"]],
  ["[主页](https://example.edu)。", ["主页。"]],
  ["曾获奖。\n\n发表过论文", ["曾获奖。", "发表过论文"]],
  ["我有**3 篇论文**，见\\\n[主页](https://example.edu)", ["我有3 篇论文，见", "主页"]],
])("marks a quote of the Markdown source as the editor shows it: %s", async (quote, marked) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <LetterEditor
          id="letter"
          markdown={"王老师：我有**3 篇论文**，见\\\n[主页](https://example.edu)。\n\n曾获奖。\n\n发表过论文。"}
          placeholders={[]}
          quotes={[quote]}
          disabled={false}
          onChange={() => {}}
        />,
      ),
    );
    const marks = Array.from(container.querySelectorAll(".tiptap .bg-warning-soft"), (node) => node.textContent);
    // A mark is cut into several elements where formatting starts or ends.
    expect(marks.join("")).toBe(marked.join(""));
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

it("imports a Word file's HTML as the letter would keep it pasted", () => {
  const html = `<h1>申请信</h1>
    <p>{导师姓名}老师：<strong>您好</strong>，见<a href="https://example.com">主页</a>和<a href="javascript:alert(1)">这里</a>。</p>`;
  expect(pastedMarkdown(html)).toBe("申请信\n\n{导师姓名}老师：**您好**，见[主页](https://example.com)和这里。");
});

it("takes a focus asked for while the cursor is already in the letter as done, so a later rebuild leaves it", async () => {
  const container = document.createElement("div");
  const elsewhere = document.createElement("input");
  document.body.append(container, elsewhere);
  const root = createRoot(container);
  const button = (name: string) => Array.from(container.querySelectorAll("button")).find((b) => b.textContent === name);
  try {
    await act(async () => root.render(<Asking />));
    const body = container.querySelector<TiptapEditorHTMLElement>(".tiptap");
    // The cursor put in the letter as by the user; the editor takes the focus in the next frame.
    act(() => {
      body?.editor?.commands.focus();
    });
    await vi.waitFor(() => expect(document.activeElement).toBe(body));
    await act(async () => button("聚焦")?.click());
    act(() => elsewhere.focus());
    await act(async () => button("重建")?.click());
    // A new editor applies its autofocus in a timer it sets once mounted.
    await act(() => new Promise((resolve) => setTimeout(resolve)));
    expect(container.querySelector(".tiptap")).not.toBe(body);
    expect(document.activeElement).toBe(elsewhere);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    elsewhere.remove();
  }
});
