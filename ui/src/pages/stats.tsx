import { useEffect, useRef, useState } from "react";
import { keepPreviousData, useQuery, type UseQueryResult } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { BackButton } from "@/back";
import { cn } from "cn";
import { call, failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type {
  ContactProgress,
  Costs,
  MentorOverview,
  ModelCall,
  ModelCallPage,
  Range,
  Trend,
  TokenTotals,
  TokenUsage,
} from "@/api/types";
import { Legend, type Point, type Series, TrendChart } from "@/components/trend-chart";
import { Pager, useLastPage } from "@/components/mentor-table";
import { crawlName, findSearch } from "@/pages/find";
import { isPurpose, PURPOSES } from "@/pages/stats-search";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { PillSelect } from "@/components/ui/pill";
import { useKeptScroll } from "@/kept-scroll";
import { mentorSearch } from "@/pages/mentors";
import { Segments } from "@/components/ui/segments";
import { Tooltip } from "@/components/ui/tooltip";
import { Loading } from "@/components/loading";
import { day, money, time } from "@/format";

// docs/ui.md 统计. Only the contact progress and token sections follow the range.

const RANGES: [Range, string][] = [
  ["week", "7 天"],
  ["month", "30 天"],
  ["quarter", "90 天"],
  ["all", "全部"],
];
const TREND: readonly Series[] = [
  { name: "发出", unit: "封", color: "var(--chart-1)" },
  { name: "回信", unit: "封", color: "var(--chart-2)" },
];
const SCHOOLS_PER_PAGE = 10;

function savedRange(): Range {
  const saved = localStorage.getItem("statsRange");
  return RANGES.find(([r]) => r === saved)?.[0] ?? "month";
}

export function StatsPage() {
  const navigate = useNavigate({ from: "/stats" });
  const [range, setRange] = useState(savedRange);
  const [now, setNow] = useState(Date.now);
  const [page, setPage] = useState(1);
  const timezone = new Intl.DateTimeFormat().resolvedOptions().timeZone;
  const period = { range, now, timezone };
  // The ranges end today: a page left open past midnight moves on with the date.
  useEffect(() => {
    const timer = setInterval(() => {
      if (new Date().toDateString() !== new Date(now).toDateString()) {
        setNow(Date.now());
        setPage(1);
        void navigate({ search: (s) => ({ ...s, callPage: undefined }), replace: true });
      }
    }, 60_000);
    return () => clearInterval(timer);
  }, [now, navigate]);
  const contact = useQuery({
    queryKey: ["stats", "contact", range, now, timezone, page],
    queryFn: () => call("contactStats", { ...period, page }),
    // Turning a page of schools keeps the rest in place; another range is drawn afresh.
    placeholderData: (previous, last) =>
      last?.queryKey[2] === range && last.queryKey[3] === now ? previous : undefined,
  });
  const mentors = useQuery({ queryKey: ["stats", "mentors"], queryFn: () => call("mentorStats", {}) });
  const usage = useQuery({ queryKey: ["stats", "tokens", period], queryFn: () => call("tokenStats", period) });
  const search = useSearch({ from: "/stats" });
  const callQuery = {
    ...period,
    purpose: search.purpose ?? null,
    model: search.model ?? null,
    page: search.callPage ?? 1,
  };
  const calls = useQuery({
    queryKey: ["stats", "calls", callQuery],
    queryFn: () => call("listModelCalls", callQuery),
    enabled: search.calls === true,
    placeholderData: keepPreviousData,
  });
  // Back from a mentor opened from the list of calls, where the page was, once all is in.
  const scroller = useKeptScroll<HTMLDivElement>(
    {
      isSuccess:
        contact.isSuccess && mentors.isSuccess && usage.isSuccess && (search.calls !== true || calls.isSuccess),
      isPlaceholderData: contact.isPlaceholderData || calls.isPlaceholderData,
    },
    "stats",
  );
  return (
    <div ref={scroller} className="@container flex-1 overflow-y-auto">
      <header className="mx-auto flex w-full max-w-[1120px] flex-wrap items-center gap-x-4 gap-y-3 px-7 pt-6 pb-2 phone:px-4 phone:pt-3.5">
        <div className="flex grow items-center gap-1">
          <BackButton
            up={() => void navigate({ to: "/more", replace: true })}
            className="-ml-2 hidden phone:inline-flex"
          />
          <h1 className="text-2xl font-semibold phone:text-[22px]">统计</h1>
        </div>
        <Segments
          label="时间范围"
          value={range}
          options={RANGES}
          onChange={(r) => {
            localStorage.setItem("statsRange", r);
            setRange(r);
            setNow(Date.now());
            setPage(1);
            void navigate({ search: (s) => ({ ...s, callPage: undefined }), replace: true });
          }}
        />
      </header>
      <div className="mx-auto w-full max-w-[1120px] space-y-12 px-7 pt-4 pb-12 phone:space-y-10 phone:px-4">
        <Section
          title="联系进度"
          note={range === "all" ? "全部时间" : `最近 ${RANGES.find(([r]) => r === range)?.[1]}`}
        >
          {/* docs/ui.md#动效: another range comes in like another tab; turning a page of schools does not. */}
          <Loaded query={contact}>
            {(c) => (
              <div key={range} className="animate-enter">
                <Contact progress={c} range={range} page={page} onPage={setPage} />
              </div>
            )}
          </Loaded>
        </Section>
        <Section title="导师" note="现有的全部导师，不含回收站">
          <Loaded query={mentors}>{(m) => <Mentors overview={m} />}</Loaded>
        </Section>
        <Section title="Token 用量" note="按模型配置里填的单价估算费用">
          <Loaded query={usage}>
            {(t) => (
              <div key={range} className="animate-enter">
                <Tokens usage={t} />
              </div>
            )}
          </Loaded>
          <Calls calls={calls} />
        </Section>
      </div>
    </div>
  );
}

function Section({ title, note, children }: { title: string; note: string; children: React.ReactNode }) {
  return (
    <section aria-label={title}>
      <div className="mb-5 flex items-baseline gap-3">
        <h2 className="text-lg font-semibold">{title}</h2>
        <p className="text-sm text-muted-foreground">{note}</p>
      </div>
      {children}
    </section>
  );
}

function Loaded<T>({
  query,
  children,
}: {
  query: { data: T | undefined; error: Error | null };
  children: (data: T) => React.ReactNode;
}) {
  if (query.error) return <FailureNote className="text-sm text-destructive" failure={failureOf(query.error)} />;
  return query.data === undefined ? (
    <div className="flex h-40">
      <Loading />
    </div>
  ) : (
    children(query.data)
  );
}

function Contact({
  progress: p,
  range,
  page,
  onPage,
}: {
  progress: ContactProgress;
  range: Range;
  page: number;
  onPage: (page: number) => void;
}) {
  if (p.contacted === 0 && p.trend.every((t) => t.sent === 0 && t.replies === 0))
    return (
      <Empty>
        这段时间尚未发出邮件。可到
        <Link to="/tasks" className="mx-1 text-brand-text hover:underline">
          任务
        </Link>
        里写首封。
      </Empty>
    );
  const days = p.medianDays === null ? null : Math.max(1, Math.ceil(p.medianDays));
  return (
    <div className="space-y-10">
      <div className="grid grid-cols-4 rounded-xl bg-muted py-4 @max-[640px]:grid-cols-2 @max-[640px]:gap-y-4">
        <Figure label="已联系" value={p.contacted} unit="位" detail="首封在这段时间发出" />
        <Figure
          label="已回复"
          value={p.replied}
          unit="位"
          detail={`回复率 ${Math.round((p.replied / Math.max(1, p.contacted)) * 100)}%`}
        />
        <Figure
          label="首次回复"
          value={days ?? "—"}
          unit={days === null ? "" : "天内"}
          detail={days === null ? "尚无回信" : "一半的导师在这个时间内回复"}
        />
        <Figure label="退信" value={p.bounced} unit="封" detail="这段时间发出后被退回" />
      </div>
      <TrendView trend={p.trend} monthly={range === "all"} />
      <div className="grid grid-cols-[2fr_3fr] gap-10 @max-[780px]:grid-cols-1">
        <div>
          <h3 className="mb-1 font-medium">首次回复用时</h3>
          <p className="mb-4 text-sm text-muted-foreground">这段时间联系、已经回复的 {p.replied} 位导师</p>
          <Bars
            rows={[
              { label: "1 天内", value: p.waiting[0] },
              { label: "1～3 天", value: p.waiting[1] },
              { label: "3～7 天", value: p.waiting[2] },
              { label: "1～2 周", value: p.waiting[3] },
              { label: "2 周以上", value: p.waiting[4] },
            ]}
            unit="位"
          />
        </div>
        <div>
          <h3 className="mb-1 font-medium">院校对比</h3>
          <p className="mb-3 text-sm text-muted-foreground">按联系的导师数排列，导师在几所学校任职的各算一次</p>
          <table className="w-full text-sm">
            <thead className="text-left text-muted-foreground">
              <tr className="border-b border-border">
                <th className="py-2 font-normal">学校</th>
                <th className="w-20 py-2 text-right font-normal">已联系</th>
                <th className="w-20 py-2 text-right font-normal">已回复</th>
                <th className="w-20 py-2 text-right font-normal">回复率</th>
              </tr>
            </thead>
            <tbody>
              {p.schools.map((s) => (
                <tr key={s.university} className="border-b border-border">
                  <td className="py-2.5 pr-3">{s.university}</td>
                  <td className="py-2.5 text-right tabular-nums">{s.contacted}</td>
                  <td className="py-2.5 text-right tabular-nums">{s.replied}</td>
                  <td className="py-2.5 text-right tabular-nums">
                    {Math.round((s.replied / Math.max(1, s.contacted)) * 100)}%
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {p.schoolCount === 0 && <p className="py-6 text-sm text-muted-foreground">联系过的导师均未填写学校</p>}
          <div className="mt-3 flex justify-end empty:hidden">
            <Pager page={page} total={p.schoolCount} pageSize={SCHOOLS_PER_PAGE} onPage={onPage} />
          </div>
        </div>
      </div>
    </div>
  );
}

function Figure({
  label,
  value,
  unit,
  detail,
}: {
  label: string;
  value: number | string;
  unit: string;
  detail: string;
}) {
  return (
    <div className="border-l border-border px-5 first:border-l-0 @max-[640px]:nth-3:border-l-0">
      <p className="text-sm text-muted-foreground">{label}</p>
      <p className="mt-1 text-[28px] leading-9 font-semibold">
        {value}
        {unit && <span className="ml-1 text-sm font-normal text-muted-foreground">{unit}</span>}
      </p>
      <p className="mt-0.5 text-xs text-muted-foreground">{detail}</p>
    </div>
  );
}

function TrendView({ trend, monthly }: { trend: Trend[]; monthly: boolean }) {
  const [table, setTable] = useState(false);
  const points: Point[] = trend.map((t) => ({ ...dateLabels(t.date, monthly), values: [t.sent, t.replies] }));
  const title = `${monthly ? "每月" : "每天"}发出的信和收到的回信`;
  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2">
        <h3 className="grow font-medium">{title}</h3>
        <Legend series={TREND} />
        <button className="text-sm text-brand-text hover:underline" onClick={() => setTable(!table)}>
          {table ? "显示图表" : "显示表格"}
        </button>
      </div>
      {table ? (
        <div className="max-h-72 overflow-y-auto">
          {/* docs/ui.md#控件: short columns share the width evenly. */}
          <table className="w-full table-fixed text-sm">
            <thead className="sticky top-0 bg-background text-left text-muted-foreground">
              <tr className="border-b border-border">
                <th className="py-2 font-normal">{monthly ? "月份" : "日期"}</th>
                <th className="py-2 text-right font-normal">发出（封）</th>
                <th className="py-2 text-right font-normal">回信（封）</th>
              </tr>
            </thead>
            <tbody>
              {points.toReversed().map((p) => (
                <tr key={p.title} className="border-b border-border">
                  <td className="py-2">{p.title}</td>
                  <td className="py-2 text-right tabular-nums">{p.values[0]}</td>
                  <td className="py-2 text-right tabular-nums">{p.values[1]}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <TrendChart series={TREND} points={points} caption={title} />
      )}
    </div>
  );
}

/** "10月25日" or "2026年10月", with the year on axis labels only where it changes. */
function dateLabels(date: string, monthly: boolean): { label: string; title: string } {
  const [y, m, d] = date.split("-").map(Number);
  if (monthly) return { label: m === 1 ? `${y}年1月` : `${m}月`, title: `${y}年${m}月` };
  return { label: `${m}月${d}日`, title: `${y}年${m}月${d}日` };
}

function Mentors({ overview: o }: { overview: MentorOverview }) {
  if (o.total === 0)
    return (
      <Empty>
        尚无导师，可去
        <Link to="/find" search={findSearch({})} className="mx-1 text-brand-text hover:underline">
          找导师
        </Link>
        整理或添加
      </Empty>
    );
  return (
    <div className="grid grid-cols-3 gap-10 @max-[900px]:grid-cols-1">
      <div>
        <h3 className="mb-4 font-medium">匹配度</h3>
        <Bars
          rows={[
            { label: "80～100 分", value: o.matching[4] },
            { label: "60～79 分", value: o.matching[3] },
            { label: "40～59 分", value: o.matching[2] },
            { label: "20～39 分", value: o.matching[1] },
            { label: "0～19 分", value: o.matching[0] },
          ]}
          unit="位"
        />
        {o.matching[5] > 0 && <p className="mt-3 text-sm text-muted-foreground">另有 {o.matching[5]} 位未分析</p>}
      </div>
      <div>
        <h3 className="mb-4 font-medium">学校</h3>
        <Bars rows={o.schools.map((s) => ({ label: s.university, value: s.count }))} unit="位" />
        {o.otherSchools > 0 && <p className="mt-3 text-sm text-muted-foreground">其他学校共 {o.otherSchools} 人次</p>}
      </div>
      <div>
        <h3 className="mb-4 font-medium">资料完整度</h3>
        <Bars
          rows={(
            [
              ["有邮箱", o.email, "email"],
              ["有研究方向", o.research, "research"],
              ["有代表论文", o.papers, "papers"],
              ["有主页", o.homepage, "homepage"],
            ] as const
          ).map(([label, value, missing]) => ({
            label,
            value,
            after: value < o.total && (
              <Link
                to="/mentors"
                search={mentorSearch({ missing })}
                className="text-brand-text tabular-nums hover:underline"
              >
                缺 {o.total - value} 位
              </Link>
            ),
          }))}
          max={o.total}
          unit="位"
          share
        />
        <p className="mt-3 text-sm text-muted-foreground">共 {o.total} 位导师</p>
      </div>
    </div>
  );
}

/**
 * A list of bars: each row its name and count, a thin bar under them growing in from the left, so no
 * axis or table is needed (docs/ui.md#图表).
 */
function Bars({
  rows,
  unit,
  max = Math.max(1, ...rows.map((r) => r.value)),
  share = false,
}: {
  /** `after` follows the count, as a link to the mentors behind it. */
  rows: { label: string; value: number; after?: React.ReactNode }[];
  unit: string;
  max?: number;
  share?: boolean;
}) {
  return (
    <ul className="space-y-3.5 text-sm">
      {rows.map((r) => (
        <li key={r.label}>
          <div className="flex items-baseline gap-3">
            <Tooltip content={r.label}>
              <span className="min-w-0 grow truncate">{r.label}</span>
            </Tooltip>
            <span className="shrink-0 text-muted-foreground tabular-nums">
              {share ? `${Math.round((r.value / max) * 100)}%` : `${r.value} ${unit}`}
            </span>
            {r.after !== undefined && <span className="w-16 shrink-0 text-right">{r.after}</span>}
          </div>
          <div className="mt-1.5 h-1.5 rounded-full bg-muted">
            <div
              className="h-full origin-left animate-grow rounded-full bg-chart-1 transition-[width] duration-240"
              style={{ width: `${(r.value / max) * 100}%` }}
            />
          </div>
        </li>
      ))}
    </ul>
  );
}

function Tokens({ usage }: { usage: TokenUsage }) {
  if (usage.purposes.length === 0) return <Empty>这段时间无 Token 用量</Empty>;
  const unpriced = usage.total.unpricedCalls;
  return (
    <>
      {/* docs/ui.md#控件: short columns share the width evenly. */}
      <table className="w-full table-fixed text-sm">
        <thead className="text-left text-muted-foreground">
          <tr className="border-b border-border">
            <th className="py-2 font-normal">用途</th>
            <th className="py-2 text-right font-normal">输入</th>
            <th className="py-2 text-right font-normal phone:hidden">其中缓存命中</th>
            <th className="py-2 text-right font-normal">输出</th>
            <th className="py-2 text-right font-normal">估算费用</th>
          </tr>
        </thead>
        <tbody>
          {usage.purposes.map((u) => (
            <TokenRow key={u.purpose} label={PURPOSES[u.purpose]} row={u} />
          ))}
          <TokenRow label="合计" row={usage.total} total />
        </tbody>
      </table>
      {unpriced > 0 && (
        <p className="mt-3 text-sm text-muted-foreground">有 {unpriced} 次调用的模型配置未填单价或已删除，未计入费用</p>
      )}
    </>
  );
}

function TokenRow({ label, row, total = false }: { label: string; row: TokenTotals; total?: boolean }) {
  return (
    <tr className={cn("border-b border-border", total && "border-b-0 font-medium")}>
      <td className="py-2.5">{label}</td>
      <td className="py-2.5 text-right tabular-nums">{tokens(row.input)}</td>
      <td className="py-2.5 text-right text-muted-foreground tabular-nums phone:hidden">{tokens(row.cached)}</td>
      <td className="py-2.5 text-right tabular-nums">{tokens(row.output)}</td>
      <td className="py-2.5 text-right tabular-nums">{cost(row.estimatedCost)}</td>
    </tr>
  );
}

/** docs/ui.md 统计: every call to a model in the range, folded away until opened. */
function Calls({ calls }: { calls: UseQueryResult<ModelCallPage> }) {
  const s = useSearch({ from: "/stats" });
  const navigate = useNavigate({ from: "/stats" });
  const open = s.calls === true;
  const page = s.callPage ?? 1;
  const crawls = useQuery({
    queryKey: ["crawlOptions"],
    queryFn: () => call("crawlOptions", {}),
    enabled: open,
  });
  const crawlNames = new Map(crawls.data?.map((c) => [c.id, crawlName(c)]));
  // One chosen before the range changed stays to be taken off.
  const models = [...new Set([...(calls.data?.models ?? []), ...(s.model === undefined ? [] : [s.model])])];
  const show = (part: typeof s) => void navigate({ search: (old) => ({ ...old, ...part }), replace: true });
  useLastPage(calls.isSuccess && !calls.isFetching ? calls.data.total : undefined, page, (n) =>
    show({ callPage: n > 1 ? n : undefined }),
  );
  const filtered = s.purpose !== undefined || s.model !== undefined;
  const top = useRef<HTMLDivElement>(null);
  return (
    <Collapsible
      ref={top}
      className="mt-6 scroll-mt-4"
      open={open}
      onOpenChange={(o) => show({ calls: o ? true : undefined })}
    >
      <CollapsibleTrigger>调用记录</CollapsibleTrigger>
      <CollapsibleContent>
        <div className="space-y-3 pt-3">
          <div className="flex flex-wrap gap-2">
            <PillSelect
              label="用途"
              options={[
                [undefined, "不限"],
                ...Object.keys(PURPOSES)
                  .filter(isPurpose)
                  .map((p) => [p, PURPOSES[p]] as const),
              ]}
              value={s.purpose}
              active={s.purpose !== undefined}
              onChange={(purpose) => show({ purpose, callPage: undefined })}
            />
            <PillSelect
              label="模型"
              options={[[undefined, "不限"], ...models.map((m) => [m, m] as const)]}
              value={s.model}
              active={s.model !== undefined}
              onChange={(model) => show({ model, callPage: undefined })}
            />
          </div>
          {calls.error ? (
            <FailureNote className="text-sm text-destructive" failure={failureOf(calls.error)} />
          ) : !calls.data ? (
            <div className="flex h-24">
              <Loading />
            </div>
          ) : calls.data.total === 0 ? (
            <Empty>{filtered ? "无符合条件的调用，可更换筛选条件查看" : "这段时间未调用模型"}</Empty>
          ) : (
            <>
              {/* Two lines each where the columns would leave the names no room, as on a phone. */}
              <table className="w-full table-fixed text-sm @max-[900px]:hidden">
                <thead className="text-left text-muted-foreground">
                  <tr className="border-b border-border">
                    <th className="w-28 py-2 font-normal">时间</th>
                    <th className="w-20 py-2 font-normal">用途</th>
                    <th className="py-2 font-normal">对象</th>
                    <th className="py-2 font-normal">模型</th>
                    <th className="w-24 py-2 text-right font-normal">输入</th>
                    <th className="w-24 py-2 text-right font-normal">其中缓存命中</th>
                    <th className="w-24 py-2 text-right font-normal">输出</th>
                    <th className="w-28 py-2 text-right font-normal">估算费用</th>
                  </tr>
                </thead>
                <tbody>
                  {calls.data.calls.map((c) => (
                    <tr key={c.id} className="border-b border-border align-top last:border-b-0">
                      <td className="py-2.5 text-muted-foreground tabular-nums">{`${day(c.at)} ${time(c.at)}`}</td>
                      <td className="py-2.5">{PURPOSES[c.purpose]}</td>
                      <td className="truncate py-2.5 pr-3">
                        <Subject call={c} crawls={crawlNames} />
                      </td>
                      <td className="py-2.5 pr-3">
                        <Tooltip content={c.model}>
                          <p className="truncate">{c.model}</p>
                        </Tooltip>
                      </td>
                      <Result call={c} />
                    </tr>
                  ))}
                </tbody>
              </table>
              <ul className="hidden divide-y divide-border text-sm @max-[900px]:block">
                {calls.data.calls.map((c) => (
                  <li key={c.id} className="space-y-1 py-2.5">
                    <div className="flex gap-3">
                      <p className="min-w-0 grow truncate">
                        {PURPOSES[c.purpose]} <Subject call={c} crawls={crawlNames} />
                      </p>
                      {c.result.kind === "answered" && (
                        <span className="shrink-0 tabular-nums">{cost(c.result.cost)}</span>
                      )}
                    </div>
                    <div className="flex gap-3 text-muted-foreground">
                      <p className="min-w-0 grow truncate">{`${day(c.at)} ${time(c.at)} · ${c.model}`}</p>
                      {c.result.kind === "answered" && c.result.tokens && (
                        <span className="shrink-0 tabular-nums">
                          输入 {tokens(c.result.tokens.input)} / 输出 {tokens(c.result.tokens.output)}
                        </span>
                      )}
                    </div>
                    {c.result.kind === "answered" && !c.result.tokens && (
                      <p className="text-muted-foreground">服务商未返回用量</p>
                    )}
                    {c.result.kind === "failed" && (
                      <FailureNote className="text-destructive" lead="失败：" failure={c.result.failure} />
                    )}
                  </li>
                ))}
              </ul>
              <div className="flex justify-end">
                <Pager
                  page={page}
                  total={calls.data.total}
                  onPage={(n) => {
                    // The next page starts at its first call, not down where these buttons are.
                    top.current?.scrollIntoView({ block: "start" });
                    show({ callPage: n > 1 ? n : undefined });
                  }}
                />
              </div>
            </>
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/**
 * Whom or what a call was for, which opens on a click; nothing once deleted. Cut short where it does
 * not fit, as a crawl named by its address may not, and whole on hover.
 */
function Subject({ call: c, crawls }: { call: ModelCall; crawls: ReadonlyMap<number, string> }) {
  if (c.mentor)
    return (
      <Tooltip content={c.mentor.name}>
        <Link to="/mentors/$mentorId" params={{ mentorId: c.mentor.id }} className="text-brand-text hover:underline">
          {c.mentor.name}
        </Link>
      </Tooltip>
    );
  const crawl = c.crawlId === null ? undefined : crawls.get(c.crawlId);
  return crawl === undefined || c.crawlId === null ? null : (
    <Tooltip content={crawl}>
      <Link
        to="/find"
        search={findSearch({ tab: "crawl", detail: c.crawlId })}
        className="text-brand-text hover:underline"
      >
        {crawl}
      </Link>
    </Tooltip>
  );
}

/** The tokens and cost of an answered call, or why it failed, across those four columns. */
function Result({ call: c }: { call: ModelCall }) {
  if (c.result.kind === "failed")
    return (
      <td colSpan={4} className="py-2.5">
        <FailureNote className="text-destructive" lead="失败：" failure={c.result.failure} />
      </td>
    );
  const { tokens: t, cost: estimated } = c.result;
  if (!t)
    return (
      <td colSpan={4} className="py-2.5 text-right text-muted-foreground">
        服务商未返回用量
      </td>
    );
  return (
    <>
      <td className="py-2.5 text-right tabular-nums">{tokens(t.input)}</td>
      <td className="py-2.5 text-right text-muted-foreground tabular-nums">{tokens(t.cached)}</td>
      <td className="py-2.5 text-right tabular-nums">{tokens(t.output)}</td>
      <td className="py-2.5 text-right tabular-nums">{cost(estimated)}</td>
    </>
  );
}

function tokens(n: number): string {
  return n.toLocaleString();
}

/** Each currency apart, as they are not converted: "3.20 元 + 0.45 美元". */
function cost({ cny, usd }: Costs): string {
  const amounts = [cny === null ? [] : [money(cny, "cny")], usd === null ? [] : [money(usd, "usd")]].flat();
  return amounts.length > 0 ? amounts.join(" + ") : "—";
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="rounded-xl bg-muted px-5 py-8 text-center text-sm text-muted-foreground">{children}</p>;
}
