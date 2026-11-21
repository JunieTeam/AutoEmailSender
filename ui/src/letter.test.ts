// @vitest-environment happy-dom
import { expect, test } from "vitest";
import type { LetterStyle } from "@/api/types";
import { letterHtml, letterText } from "@/letter";

const style: LetterStyle = {
  font: null,
  fontSizePt: 10.5,
  lineHeight: 1.6,
  paragraphSpacingEm: 0.8,
  firstLineIndent: false,
};
const render = (markdown: string, s: LetterStyle) =>
  new DOMParser().parseFromString(letterHtml(markdown, s), "text/html").body;

test("the font is left to the recipient unless one is chosen", () => {
  expect(render("甲", style).querySelector("div")?.style.fontFamily).toBe("");
  expect(render("甲", { ...style, font: "song" }).querySelector("div")?.style.fontFamily).toContain("SimSun");
});

test("paragraphs are indented, and lists move in as a whole", () => {
  const page = render("甲\n\n1. 乙\n\n   丙\n\n   - 戊\n\n丁", { ...style, firstLineIndent: true });
  const indents = [...page.querySelectorAll("p")].map((p) => [p.textContent, p.style.textIndent]);
  expect(indents).toEqual([
    ["甲", "2em"],
    ["乙", ""],
    ["丙", ""],
    ["丁", "2em"],
  ]);
  expect([...page.querySelectorAll<HTMLElement>("ol, ul")].map((l) => l.style.paddingLeft)).toEqual(["4em", "2em"]);
  const plain = render("甲\n\n- 乙", style);
  expect(plain.querySelector("ul")?.style.paddingLeft).toBe("2em");
});

const paragraphs = (markdown: string, s: LetterStyle) =>
  [...render(markdown, s).querySelectorAll("p")].map((p) => [p.textContent.trim(), p.style.textIndent]);

test("an indented letter leaves its salutation and a 敬礼 standing alone at the margin", () => {
  const indented = { ...style, firstLineIndent: true };
  expect(paragraphs("王老师：\n您好！我是林晓。\n\n此致\n敬礼！\n\n林晓", indented)).toEqual([
    ["王老师：", ""],
    ["您好！我是林晓。", "2em"],
    ["此致敬礼！", "2em"],
    ["林晓", "2em"],
  ]);
  expect(paragraphs("尊敬的王老师:\n\n正文：\n\n此致\n\n敬礼", indented)).toEqual([
    ["尊敬的王老师:", ""],
    ["正文：", "2em"],
    ["此致", "2em"],
    ["敬礼", ""],
  ]);
  expect(paragraphs("您好！我是林晓。\n\n下面是：", indented)).toEqual([
    ["您好！我是林晓。", "2em"],
    ["下面是：", "2em"],
  ]);
  expect(paragraphs("王老师：\n您好！", style)).toEqual([["王老师：您好！", ""]]);
});

test("links to anything but web pages and email addresses are only text", () => {
  const page = render("[点我](javascript:alert(1)) [网页](https://a.com) [邮箱](mailto:a@b.com)", style);
  expect([...page.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
    "https://a.com",
    "mailto:a@b.com",
  ]);
  expect(page.textContent).toContain("点我");
});

test("pictures and task boxes stay as written", () => {
  const page = render("![简历](https://a.com/t.png)\n\n- [x] 做过的工作\n\n  补充说明", style);
  expect(page.querySelector("img, input")).toBeNull();
  expect([...page.querySelectorAll("p")].map((e) => e.textContent)).toEqual([
    "![简历](https://a.com/t.png)",
    "[x] 做过的工作",
    "补充说明",
  ]);
});

test("spaces show as typed", () => {
  const page = render("    王老师：\n\n张三    计算机专业\\\n  **large**  language **models** too", style);
  const shown = [...page.querySelectorAll("p")].map((p) => p.innerHTML);
  expect(shown).toEqual([
    "&nbsp;&nbsp;&nbsp;&nbsp;王老师：",
    "张三 &nbsp;&nbsp;&nbsp;计算机专业<br>&nbsp;&nbsp;<strong>large</strong> &nbsp;language <strong>models</strong> too",
  ]);
});

test("the plain text keeps lines, lists and link addresses", () => {
  const markdown = [
    "王老师：",
    "您好！我是**林晓**。\n此致\n敬礼",
    "- 甲\n  - 乙\n\n  丁\n- 丙",
    "3. 丁\n\n   戊\n4. 己",
    "主页 [linxiao.dev](https://linxiao.dev)，邮箱 [me@x.com](mailto:me@x.com)",
  ].join("\n\n");
  expect(letterText(markdown)).toBe(
    [
      "王老师：",
      "您好！我是林晓。\n此致\n敬礼",
      "- 甲\n  - 乙\n  丁\n- 丙",
      "3. 丁\n   戊\n4. 己",
      "主页 linxiao.dev <https://linxiao.dev>，邮箱 me@x.com",
    ].join("\n\n"),
  );
});
