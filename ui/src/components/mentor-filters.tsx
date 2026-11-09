import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { useState } from "react";
import { call, failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type { MentorFilter, Missing } from "@/api/types";
import { Loading } from "@/components/loading";
import { useTags } from "@/components/tags";
import { scrollsSideways } from "@/sideways";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { PillLabel, pillClass, PillSelect } from "@/components/ui/pill";
import { MENU_ITEM, Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

/** docs/ui.md#交互: the filters under the contact states, as they are kept in the URL. */
export type FilterSearch = {
  match?: Match;
  missing?: Missing;
  university?: (string | null)[];
  school?: (string | null)[];
  department?: (string | null)[];
  title?: (string | null)[];
  tag?: (number | null)[];
};

type Match = 80 | 60 | 40 | "none";

const MATCHES: [Match | undefined, string][] = [
  [undefined, "不限"],
  [80, "80 分以上"],
  [60, "60 分以上"],
  [40, "40 分以上"],
  ["none", "未分析"],
];

export const MISSING: [Missing | undefined, string][] = [
  [undefined, "不限"],
  ["email", "缺邮箱"],
  ["research", "缺研究方向"],
  ["papers", "缺代表论文"],
  ["homepage", "缺主页"],
];

const FIELDS = [
  ["university", "学校", "universities"],
  ["school", "学院", "schools"],
  ["department", "系所", "departments"],
  ["title", "职称", "titles"],
] as const;

/** The chosen values of a field, or undefined when none is chosen. */
function texts(v: unknown) {
  const values = Array.isArray(v)
    ? v.filter((x): x is string | null => x === null || (typeof x === "string" && x.trim() !== ""))
    : [];
  return values.length > 0 ? values : undefined;
}

/** Anything unexpected in the URL is dropped, as for the other filters. */
export function filterSearch(search: Record<string, unknown>): FilterSearch {
  const tags = Array.isArray(search.tag)
    ? search.tag.filter((x): x is number | null => x === null || Number.isSafeInteger(x))
    : [];
  return {
    match: MATCHES.find(([m]) => m !== undefined && m === search.match)?.[0],
    missing: MISSING.find(([m]) => m !== undefined && m === search.missing)?.[0],
    university: texts(search.university),
    school: texts(search.school),
    department: texts(search.department),
    title: texts(search.title),
    tag: tags.length > 0 ? tags : undefined,
  };
}

/** The part of the list query these filters make up. */
export function filterOf(s: FilterSearch): MentorFilter {
  return {
    matching:
      s.match === undefined
        ? undefined
        : s.match === "none"
          ? { kind: "unanalyzed" }
          : { kind: "atLeast", score: s.match },
    universities: s.university,
    schools: s.school,
    departments: s.department,
    titles: s.title,
    tags: s.tag,
    missing: s.missing,
  };
}

const NO_FILTERS = {
  match: undefined,
  missing: undefined,
  university: undefined,
  school: undefined,
  department: undefined,
  title: undefined,
  tag: undefined,
} satisfies Record<keyof FilterSearch, undefined>;

export function filtered(s: FilterSearch) {
  return Object.values(filterOf(s)).some((v) => v !== undefined);
}

/**
 * One row of menus, `children` at its end; every change starts again from the first page. A menu
 * offers the values among the mentors the rest of `filter`, the list's, leaves.
 */
export function MentorFilters({
  search,
  filter,
  onChange,
  children,
}: {
  search: FilterSearch;
  filter: MentorFilter;
  onChange: (next: FilterSearch) => void;
  children?: React.ReactNode;
}) {
  const tags = useTags();
  return (
    <div
      ref={scrollsSideways}
      className="flex flex-wrap items-center gap-2 px-7 pb-3 phone:flex-nowrap phone:overflow-x-auto phone:px-4 phone:pb-2.5 phone:[scrollbar-width:none]"
    >
      <PillSelect
        label="匹配度"
        options={MATCHES}
        value={search.match}
        active={search.match !== undefined}
        onChange={(match) => onChange({ ...search, match })}
      />
      {FIELDS.map(([key, label, list]) => {
        // Without its own choice, so that ticking in the menu keeps what it lists.
        const others = { ...filter, [list]: undefined };
        return (
          <Choices
            key={key}
            label={label}
            selected={search[key] ?? []}
            name={(v) => v ?? "未填写"}
            queryKey={["mentorFilterValues", key, others]}
            load={async () => {
              const o = await call("mentorFilterValues", { field: key, ...others });
              return {
                choices: o.values.map((v) => ({ value: v.value, label: v.value, count: v.count })),
                missing: o.missing,
              };
            }}
            onChange={(values) => onChange({ ...search, [key]: values.length > 0 ? values : undefined })}
          />
        );
      })}
      <Choices
        label="标签"
        selected={search.tag ?? []}
        name={(id) =>
          id === null
            ? "无标签"
            : (tags.data?.find((t) => t.id === id)?.name ??
              (tags.isError ? "未能读取标签名" : tags.isPending ? "…" : "已删除的标签"))
        }
        queryKey={["mentorFilterTags", { ...filter, tags: undefined }]}
        load={async () => {
          const o = await call("mentorFilterTags", { ...filter, tags: undefined });
          return { choices: o.tags.map((t) => ({ value: t.id, label: t.name, count: t.count })), missing: o.untagged };
        }}
        onChange={(values) => onChange({ ...search, tag: values.length > 0 ? values : undefined })}
      />
      <PillSelect
        label="资料"
        options={MISSING}
        value={search.missing}
        active={search.missing !== undefined}
        onChange={(missing) => onChange({ ...search, missing })}
      />
      <div className="ml-auto flex shrink-0 items-center gap-2">
        {filtered(search) && (
          <Button variant="ghost" className="px-3" onClick={() => onChange(NO_FILTERS)}>
            清除筛选
          </Button>
        )}
        {children}
      </div>
    </div>
  );
}

type Choice<T> = { value: T; label: string; count: number };

type ChoicesProps<T> = {
  label: string;
  selected: (T | null)[];
  /** What a value not among those loaded is called; null is the mentors with none. */
  name: (value: T | null) => string;
  queryKey: unknown[];
  load: () => Promise<{ choices: Choice<T>[]; missing: number }>;
  onChange: (values: (T | null)[]) => void;
};

/** Any of the ticked values, with the mentors who have none of them at the bottom. */
function Choices<T extends string | number>(props: ChoicesProps<T>) {
  const { label, selected, name } = props;
  const [first] = selected;
  const shown =
    first === undefined ? undefined : selected.length > 1 ? `${name(first)} 等 ${selected.length} 个` : name(first);
  return (
    <Popover>
      <PopoverTrigger render={<button className={pillClass(shown !== undefined)} />}>
        <PillLabel label={label} shown={shown} />
      </PopoverTrigger>
      {/* Of a set width, so that it keeps its size while its search box narrows the list. */}
      <PopoverContent align="start" className="w-72">
        <ChoiceList {...props} />
      </PopoverContent>
    </Popover>
  );
}

/** The menu's list, loaded each time it opens, so that the counts are those of now. */
function ChoiceList<T extends string | number>({ label, selected, name, queryKey, load, onChange }: ChoicesProps<T>) {
  const query = useQuery({ queryKey, queryFn: load, placeholderData: keepPreviousData });
  const [text, setText] = useState("");
  if (query.error) {
    return <FailureNote className="px-2.5 py-2 text-destructive" lead="未能加载：" failure={failureOf(query.error)} />;
  }
  if (!query.data) return <Loading />;
  const { choices, missing } = query.data;
  const toggle = (v: T | null) => onChange(selected.includes(v) ? selected.filter((s) => s !== v) : [...selected, v]);
  // A chosen value the other filters leave no one with stays listed, so that it can be unticked.
  const gone = selected.flatMap((v) =>
    v === null || choices.some((c) => c.value === v) ? [] : [{ value: v, label: name(v), count: 0 }],
  );
  const words = text.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const listed = [...choices, ...gone].filter((c) => words.every((w) => inOrder(c.label.toLowerCase(), w)));
  const unticked = listed.filter((c) => !selected.includes(c.value));
  const none = (missing > 0 && words.length === 0) || selected.includes(null);
  return (
    <>
      {choices.length > 8 && (
        <Input
          autoFocus
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={`搜索${label}`}
          aria-label={`搜索${label}`}
          className="mb-1 h-9"
        />
      )}
      <div className="max-h-72 overflow-y-auto">
        {words.length > 0 && listed.length > 1 && unticked.length > 0 && (
          <button
            className={cn(MENU_ITEM, "text-brand-text")}
            onClick={() => onChange([...selected, ...unticked.map((c) => c.value)])}
          >
            全选搜到的 {listed.length} 项
          </button>
        )}
        {listed.map((c) => (
          <Option
            key={c.value}
            label={c.label}
            count={c.count}
            checked={selected.includes(c.value)}
            onToggle={() => toggle(c.value)}
          />
        ))}
        {none && (
          <Option
            label={name(null)}
            count={missing}
            checked={selected.includes(null)}
            onToggle={() => toggle(null)}
            className="text-muted-foreground"
          />
        )}
        {listed.length === 0 && !none && (
          <p className="px-2.5 py-2 text-sm text-muted-foreground">
            {words.length > 0 ? `没有匹配的${label}` : `没有可选的${label}`}
          </p>
        )}
      </div>
    </>
  );
}

/** Whether the characters of `query` come in `text` in order, as "北航" in "北京航空航天大学". */
function inOrder(text: string, query: string) {
  let at = 0;
  for (const c of query) {
    at = text.indexOf(c, at);
    if (at < 0) return false;
    at += c.length;
  }
  return true;
}

function Option({
  label,
  count,
  checked,
  onToggle,
  className,
}: {
  label: string;
  count: number;
  checked: boolean;
  onToggle: () => void;
  className?: string;
}) {
  return (
    <label className={cn(MENU_ITEM, className)}>
      <Checkbox checked={checked} onCheckedChange={onToggle} />
      <span className="min-w-0 grow truncate">{label}</span>
      <span className="text-xs text-muted-foreground">{count}</span>
    </label>
  );
}
