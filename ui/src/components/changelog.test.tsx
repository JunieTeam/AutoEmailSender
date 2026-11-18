// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { Release } from "@/api/types";
import { UpdateChanges } from "./changelog";

function render(releases: Release[]) {
  const host = document.createElement("div");
  act(() => createRoot(host).render(<UpdateChanges releases={releases} />));
  return {
    summary: host.querySelector("p")?.textContent,
    sections: [...host.querySelectorAll("section")].map((s) => [
      s.querySelector("h3")?.textContent,
      [...s.querySelectorAll("li")].map((li) => li.textContent),
    ]),
  };
}

it("lists an update's changes by kind, each with its version when it spans several", () => {
  const releases: Release[] = [
    {
      version: "3.1.0",
      date: "2027-01-02",
      changes: [
        { kind: "fix", text: "乙" },
        { kind: "feature", text: "甲" },
      ],
    },
    { version: "3.0.1", date: "2026-12-20", changes: [{ kind: "feature", text: "丙" }] },
  ];
  expect(render(releases)).toEqual({
    summary: "2 个版本 · 3.0.1 – 3.1.0",
    sections: [
      ["新功能 2", ["甲3.1.0", "丙3.0.1"]],
      ["修复 1", ["乙3.1.0"]],
    ],
  });
  expect(render(releases.slice(0, 1)).sections).toEqual([
    ["新功能 1", ["甲"]],
    ["修复 1", ["乙"]],
  ]);
  expect(render([]).summary).toBe("这个版本无更新说明");
});
