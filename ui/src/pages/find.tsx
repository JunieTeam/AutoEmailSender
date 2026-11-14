import {
  skipToken,
  type UseMutationResult,
  type UseQueryResult,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Link, useNavigate, useRouter, useRouterState, useSearch } from "@tanstack/react-router";
import { cn } from "cn";
import { Inbox, SearchX } from "lucide-react";
import { BackButton } from "@/back";
import { Loading } from "@/components/loading";
import { Notice } from "@/components/notice";
import { FailureDetails, FailureNote } from "@/components/failure";
import { type ReactNode, useEffect, useState } from "react";
import { exportCrawlLog } from "@/api/app";
import { call, failureOf, UiError } from "@/api/call";
import type {
  CandidateCounts,
  CandidateFilter,
  CandidatePage,
  CandidateRow,
  CandidateSelection,
  CrawlOption,
  CrawlPage,
  CrawlRun,
  CrawlStage,
  CrawlSummary,
  Decisions,
  EnrichField,
  EnrichPlan,
  EnrichSkipReason,
  ModelRate,
  PageKind,
  TokenTotals,
} from "@/api/types";
import { ModelAction, AiIcon } from "@/components/ai-writing";
import { featureModel } from "@/components/model-dialog";
import { BatchBar, Confirm } from "@/components/batch-bar";
import { Field, formClose } from "@/components/form";
import { IntakeChoices } from "@/components/intake-choices";
import { Missing, refreshMentors, placeOf } from "@/components/mentor";
import { Pager } from "@/components/mentor-table";
import { LibraryTab } from "@/pages/library";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Segments } from "@/components/ui/segments";
import { StatusLabel, type Tone } from "@/components/ui/status-dot";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { PillSelect } from "@/components/ui/pill";
import { costAt, costEstimate, day, hoursNote, money, time, UNITS } from "@/format";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader } from "@/components/ui/dialog";
import { desktop } from "@/host";
import { useKeptScroll } from "@/kept-scroll";
import { COMPACT } from "@/layout";
import { useNow } from "@/now";
import { pageKey } from "@/page-keys";
import { SearchBox } from "@/components/search-box";
import { Tabs } from "@/components/tabs";

const FILTERS = {
  all: "全部",
  new: "新导师",
  supplement: "可补充已有导师",
  maybe: "可能是同一位",
  missing_email: "缺邮箱",
} as const;
const EMPTY: Decisions = { take: [], matches: [], restore: [] };
const TABS = ["candidates", "crawl", "library"] as const;
const TAB_NAMES: Record<(typeof TABS)[number], string> = { candidates: "候选", crawl: "智能整理", library: "导师库" };
const STAGES: Record<CrawlStage, [string, Tone]> = {
  lists: ["正在找名单", "info"],
  profiles: ["正在读个人页", "info"],
  enrich: ["正在补全信息", "info"],
  paused: ["已暂停", "warning"],
  done: ["已完成", "success"],
  failed: ["未能完成", "destructive"],
  canceled: ["已取消", "faint"],
};
/** What reading a page for a mentor takes, as measured on school sites. */
const PAGE = { input: 1800, output: 200 };
const PAGE_TOKENS = PAGE.input + PAGE.output;
/**
 * docs/ui.md#找导师: a crawl by the school and college it found, else by where it started; an Agent's
 * submission by its school. Short enough for a menu or a line.
 */
export function crawlName(c: Pick<CrawlOption, "kind" | "url" | "university" | "school">): string {
  if (c.kind === "mentors") return "补全已有导师";
  const unit = [c.university, c.school].filter(Boolean).join(" ");
  if (c.kind === "agent") return unit ? `Agent 提交：${unit}` : "Agent 提交";
  return (
    unit ||
    (c.url ?? "")
      .replace(/^https?:\/\//, "")
      .replace(/\/$/, "")
      .slice(0, 48)
  );
}
export function findSearch(s: Record<string, unknown>) {
  const filter: CandidateFilter =
    s.filter === "new" || s.filter === "supplement" || s.filter === "maybe" || s.filter === "missing_email"
      ? s.filter
      : "all";
  return {
    tab: TABS.find((tab) => tab === s.tab) ?? "candidates",
    filter,
    crawl: Number(s.crawl) || undefined,
    processed: s.processed === true,
    /** Each tab has its own page, kept while another tab is on show. */
    page: Math.max(1, Number(s.page) || 1),
    recordPage: Math.max(1, Number(s.recordPage) || 1),
    libraryPage: Math.max(1, Number(s.libraryPage) || 1),
    detail: Number(s.detail) || undefined,
    /** The candidate on show beside the list, or over it in a narrow window. */
    candidate: Number(s.candidate) || undefined,
    org: typeof s.org === "string" && s.org !== "" ? s.org : undefined,
    q: typeof s.q === "string" && s.q !== "" ? s.q : undefined,
    /** The candidates' keyword; the library has its own. */
    keyword: typeof s.keyword === "string" && s.keyword !== "" ? s.keyword : undefined,
  };
}
export function refreshCrawls(client: ReturnType<typeof useQueryClient>) {
  for (const key of [
    "listCrawls",
    "crawlOptions",
    "crawlWork",
    "getCrawl",
    "crawlUsage",
    "listCandidates",
    "getCandidate",
    "previewCandidates",
    "previewEnrich",
    "previewEnrichMentors",
    "stats",
  ])
    void client.invalidateQueries({ queryKey: [key] });
}
export function FindPage() {
  const search = useSearch({ from: "/find" });
  const navigate = useNavigate({ from: "/find" });
  return (
    <>
      <header className="shrink-0 px-6 pt-6 phone:px-4">
        <div className="flex items-center gap-1">
          <BackButton
            up={() => void navigate({ to: "/more", replace: true })}
            className="-ml-2 hidden phone:inline-flex"
          />
          <h1 className="text-2xl font-semibold">找导师</h1>
        </div>
        <Segments
          label="找导师"
          className="my-4"
          value={search.tab}
          options={TABS.map((tab) => [tab, TAB_NAMES[tab]] as const)}
          onChange={(tab) => void navigate({ search: (s) => ({ ...s, tab }), replace: true })}
        />
      </header>
      <Tabs shown={search.tab}>
        {(tab) => (tab === "crawl" ? <Crawls /> : tab === "library" ? <LibraryTab /> : <Candidates />)}
      </Tabs>
    </>
  );
}
function Crawls() {
  const s = useSearch({ from: "/find" });
  const navigate = useNavigate({ from: "/find" });
  const client = useQueryClient();
  const { data: urls = "" } = useQuery({
    queryKey: ["crawlUrls"],
    queryFn: skipToken,
    initialData: "",
    gcTime: Infinity,
  });
  const setUrls = (value: string) => client.setQueryData(["crawlUrls"], value);
  const validation = useQuery({
    queryKey: ["checkCrawlUrls", urls],
    queryFn: () => call("checkCrawlUrls", { urls }),
    retry: false,
  });
  const models = useQuery({ queryKey: ["listModelConfigs"], queryFn: () => call("listModelConfigs", {}) });
  const chosen = models.data && featureModel(models.data, "crawl");
  const records = useQuery({
    queryKey: ["listCrawls", s.recordPage],
    queryFn: () => call("listCrawls", { page: s.recordPage }),
    refetchInterval: 2000,
  });
  // The address also holds the other tabs' state.
  const scroller = useKeptScroll<HTMLDivElement>(records, String(s.recordPage));
  const start = useMutation({
    mutationFn: () => {
      if (!chosen) throw new UiError("请在设置中添加模型");
      return call("startCrawl", { urls, modelId: chosen.id });
    },
    onError: () => client.invalidateQueries({ queryKey: ["listModelConfigs"] }),
    onSuccess: (r) => {
      setUrls("");
      refreshCrawls(client);
      void navigate({ search: (old) => ({ ...old, recordPage: 1, detail: r.id }), state: { opened: true } });
    },
  });
  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto px-6 pb-8 phone:px-4">
      <section className="mb-6 max-w-3xl space-y-3 rounded-lg bg-muted p-4">
        <label className="block font-medium" htmlFor="crawl-urls">
          学院官网或教师名单网址
        </label>
        <Textarea
          id="crawl-urls"
          // docs/ui.md#控件: on the panel's --muted, the field is white.
          className="bg-background"
          rows={3}
          value={urls}
          placeholder="每行一个网址，也可以填写一位导师的个人页"
          onChange={(e) => setUrls(e.target.value)}
          aria-describedby="crawl-usage crawl-url-error"
          aria-invalid={!!urls.trim() && validation.isError}
        />
        <p id="crawl-usage" className="text-sm text-muted-foreground">
          一位导师约 {PAGE_TOKENS} Token{chosen?.rate && `，${costEstimate(chosen.rate, PAGE.input, PAGE.output, 3)}`}
          。实际用量随人数和网页长度变化，另计寻找名单的用量。
        </p>
        <p className="text-sm text-muted-foreground">
          自动寻找名单和分类，读取个人页后进入候选审核。网页标题、文字、链接文字和公开的导师资料会发给模型服务商。
        </p>
        <ModelAction
          feature="crawl"
          disabled={start.isPending || !validation.data || validation.isFetching}
          onWrite={() => start.mutate()}
        >
          <AiIcon />
          {start.isPending ? "正在开始…" : "开始整理"}
        </ModelAction>
        <div id="crawl-url-error" className="text-sm text-muted-foreground">
          {urls.trim() &&
            (validation.error ? (
              <FailureNote failure={failureOf(validation.error)} />
            ) : validation.isFetching ? (
              "正在检查网址…"
            ) : null)}
        </div>
        <ErrorText error={models.error ?? start.error} />
      </section>
      <h2 className="mb-2 text-lg font-medium">整理记录</h2>
      <ErrorText error={records.error} />
      {records.isPending && <Loading />}
      {records.data?.total === 0 && <p className="text-muted-foreground">尚无整理记录，从上方填写网址开始</p>}
      <div className="max-w-3xl">
        {records.data?.crawls.map((c) => (
          <CrawlRow key={c.id} crawl={c} />
        ))}
      </div>
      <Pages
        page={s.recordPage}
        total={records.data?.total ?? 0}
        onPage={(recordPage) => void navigate({ search: (old) => ({ ...old, recordPage }), replace: true })}
      />
      {s.detail && <CrawlDialog key={s.detail} id={s.detail} />}
    </div>
  );
}
/** docs/ui.md#找导师: a crawl's state, and while it reads profiles or fills candidates in, how far it has got. */
function CrawlStatus({ crawl: c, className }: { crawl: CrawlSummary; className?: string }) {
  const [name, tone] = STAGES[c.stage];
  const label =
    c.stage === "profiles"
      ? `读个人页 ${c.profilesDone}/${c.profiles}`
      : c.stage === "enrich"
        ? `补全还剩 ${c.enrichRemaining} 位`
        : name;
  return <StatusLabel tone={tone} label={label} className={className} />;
}
/** What a crawl has found: once it stops reading for one of a website, as all along for the others. */
function found(c: CrawlSummary): string | false {
  if (c.kind === "agent") return `收到 ${c.candidates} 位导师`;
  if (c.kind === "mentors") return `补全 ${c.candidates} 位`;
  // With none found, the line of its reason says what there is to say.
  return c.state !== "running" && c.candidates > 0 && `找到 ${c.candidates} 位导师，${c.withEmail} 位有邮箱`;
}
function CrawlRow({ crawl: c }: { crawl: CrawlSummary }) {
  const navigate = useNavigate({ from: "/find" });
  const summary = found(c);
  return (
    <div className="mb-[3px] flex animate-fade-in items-center gap-2 rounded-md hover:bg-muted phone:flex-wrap">
      <button
        className="min-w-0 grow px-4 py-3 text-left"
        // docs/ui.md#交互: the details are a step, which going back closes.
        onClick={() => void navigate({ search: (s) => ({ ...s, detail: c.id }), state: { opened: true } })}
      >
        <span className="flex items-center gap-3">
          <span className="min-w-0 grow truncate font-medium">{crawlName(c)}</span>
          <CrawlStatus crawl={c} className="text-sm text-muted-foreground" />
        </span>
        <span className="mt-1 block truncate text-xs text-muted-foreground">
          {day(c.createdAt)} {time(c.createdAt)}
          {summary && ` · ${summary}`}
        </span>
        {c.error && <span className="mt-1 block truncate text-xs text-warning">{c.error.message}</span>}
      </button>
      {/* As wide on every row, so that their states line up. */}
      <div className="flex w-28 shrink-0 items-center justify-end gap-1 pr-3 phone:w-full phone:pb-2 phone:empty:hidden">
        <CrawlActions crawl={c} row />
      </div>
    </div>
  );
}
/** "继续" of a paused crawl, which reads only the pages still to read; with none left, it calls no model. */
export function ResumeCrawl({
  crawl: c,
  disabled,
  onResume,
}: {
  crawl: CrawlSummary;
  disabled: boolean;
  onResume: () => void;
}) {
  return c.remaining > 0 ? (
    <ModelAction feature="crawl" disabled={disabled} modelIds={c.modelIds} onWrite={onResume}>
      继续
    </ModelAction>
  ) : (
    <Button variant="quiet" disabled={disabled} onClick={onResume}>
      继续
    </Button>
  );
}
/** docs/ui.md#找导师: what can be done with a crawl now; its row offers only the next step, its details all. */
function CrawlActions({ crawl: c, row = false }: { crawl: CrawlSummary; row?: boolean }) {
  const client = useQueryClient();
  const navigate = useNavigate({ from: "/find" });
  const mutation = useMutation({
    mutationFn: (action: "pauseCrawl" | "resumeCrawl" | "cancelCrawl") => call(action, { id: c.id }),
    onSuccess: () => refreshCrawls(client),
    onError: () => client.invalidateQueries({ queryKey: ["listModelConfigs"] }),
  });
  const going = c.state === "running" || c.state === "paused";
  return (
    <>
      {c.state === "running" && (
        <Button variant="quiet" disabled={mutation.isPending} onClick={() => mutation.mutate("pauseCrawl")}>
          暂停
        </Button>
      )}
      {c.state === "paused" && (
        <ResumeCrawl crawl={c} disabled={mutation.isPending} onResume={() => mutation.mutate("resumeCrawl")} />
      )}
      {going && !row && (
        <Confirm
          trigger={
            <Button variant="ghost" disabled={mutation.isPending}>
              取消
            </Button>
          }
          message="取消这次整理？已找到的候选和补到的资料会保留，仍可审核。"
          action="取消整理"
          onConfirm={() => mutation.mutate("cancelCrawl")}
        />
      )}
      {c.pending > 0 && !(row && going) && (
        <Button
          variant="ghost"
          onClick={() =>
            void navigate({
              search: (s) => ({
                ...s,
                tab: "candidates",
                crawl: c.id,
                filter: "all",
                keyword: undefined,
                processed: false,
                page: 1,
                candidate: undefined,
                detail: undefined,
              }),
            })
          }
        >
          查看候选
        </Button>
      )}
      <ErrorText error={mutation.error} />
    </>
  );
}
/** docs/ui.md#找导师: all about one crawl, opened from its row as a step that going back closes. */
function CrawlDialog({ id }: { id: number }) {
  const client = useQueryClient();
  const router = useRouter();
  const navigate = useNavigate({ from: "/find" });
  const opened = useRouterState({ select: (s) => s.location.state.opened === true });
  const [editing, setEditing] = useState(false);
  // Opened from elsewhere, as from 后台, the details give way to the records in place.
  const close = () => {
    if (opened) router.history.back();
    else void navigate({ search: (s) => ({ ...s, detail: undefined }), replace: true });
  };
  const [page, setPage] = useState(1);
  const detail = useQuery({
    queryKey: ["getCrawl", id, page, true],
    queryFn: () => call("getCrawl", { id, page, failed: true }),
    refetchInterval: 2000,
  });
  const c = detail.data?.summary;
  const usage = useQuery({
    queryKey: ["crawlUsage", id],
    queryFn: () => call("crawlUsage", { id }),
    enabled: c?.kind === "site",
    refetchInterval: c?.state === "running" ? 2000 : false,
  });
  const now = useNow();
  const retry = useMutation({
    mutationFn: (target: number | null) => call("retryCrawl", { id, page: target }),
    onSuccess: () => refreshCrawls(client),
    onError: () => client.invalidateQueries({ queryKey: ["listModelConfigs"] }),
  });
  const exported = useMutation({ mutationFn: () => exportCrawlLog(id) });
  const remove = useMutation({
    mutationFn: () => call("deleteCrawl", { id }),
    onSuccess: () => {
      // The candidates may be filtered by this crawl, which is gone now.
      void navigate({
        search: (s) =>
          s.crawl === id ? { ...s, detail: undefined, crawl: undefined, page: 1 } : { ...s, detail: undefined },
        replace: true,
      });
      refreshCrawls(client);
    },
  });
  const d = detail.data;
  return (
    <Dialog open onOpenChange={formClose(editing, false, close)}>
      <DialogContent className="w-[760px]">
        <DialogHeader>{c ? crawlName(c) : "整理详情"}</DialogHeader>
        <DialogBody className="space-y-5 pb-4 text-sm">
          <ErrorText error={detail.error ?? retry.error} />
          {!d || !c ? (
            !detail.error && <Loading />
          ) : (
            <>
              {c.kind === "site" && (
                <div className="space-y-1">
                  {d.urls.map((url) => (
                    <p key={url} className="break-all text-muted-foreground select-text">
                      {url}
                    </p>
                  ))}
                  <CrawlUnit crawl={c} onEditing={setEditing} />
                </div>
              )}
              <dl className="grid grid-cols-3 gap-x-6 gap-y-4 rounded-lg bg-muted p-4 phone:grid-cols-2">
                <Figure label="状态">
                  <CrawlStatus crawl={c} />
                </Figure>
                <Figure label={c.kind === "agent" ? "收到导师" : c.kind === "mentors" ? "补全导师" : "找到导师"}>
                  {c.candidates} 位
                </Figure>
                <Figure label="有邮箱">{c.withEmail} 位</Figure>
                {c.kind === "site" && (
                  <>
                    <Figure label="个人页（已读/共）">
                      {c.profilesDone} / {c.profiles}
                    </Figure>
                    <Figure label="用量">{usage.data ? spent(usage.data) : "—"}</Figure>
                    <Figure label="用时">{elapsed(d.runs, now)}</Figure>
                  </>
                )}
              </dl>
              {c.error && <FailureNote failure={c.error} className="text-warning" />}
              <div className="flex flex-wrap items-center gap-2 empty:hidden">
                <CrawlActions crawl={c} />
              </div>
              {d.limits.length > 0 && (
                <p className="rounded-md bg-warning-soft p-3 text-warning">名单可能不完整：{d.limits.join("；")}</p>
              )}
              {d.pageTotal > 0 && (
                <section className="space-y-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="mr-auto font-medium">未打开的页面 · {d.pageTotal} 页</h3>
                    {c.retryable > 1 && (
                      <ModelAction
                        feature="crawl"
                        modelIds={c.retryModelIds}
                        disabled={retry.isPending}
                        onWrite={() => retry.mutate(null)}
                      >
                        全部重试
                      </ModelAction>
                    )}
                  </div>
                  {d.pages.map((p) => (
                    <PageLine key={p.id} page={p}>
                      {p.retryable && (
                        // A retry sets the crawl going again, so the pages still to read are read too.
                        <ModelAction
                          feature="crawl"
                          modelIds={[...c.modelIds, p.kind.startsWith("enrich_") ? null : c.modelConfigId]}
                          disabled={retry.isPending}
                          onWrite={() => retry.mutate(p.id)}
                        >
                          重试
                        </ModelAction>
                      )}
                    </PageLine>
                  ))}
                  <Pages page={page} total={d.pageTotal} onPage={setPage} />
                </section>
              )}
              {c.pages + c.remaining > 0 && <AllPages id={id} total={c.pages + c.remaining} />}
              {d.runs.length > 0 && (
                <Collapsible>
                  <CollapsibleTrigger>运行记录</CollapsibleTrigger>
                  <CollapsibleContent>
                    {d.runs.map((r, n) => (
                      <p key={n} className="mt-2 text-xs text-muted-foreground">
                        {day(r.startedAt)} {time(r.startedAt)} —{" "}
                        {r.endedAt ? `${day(r.endedAt)} ${time(r.endedAt)}` : "进行中"} · {r.appVersion}
                      </p>
                    ))}
                  </CollapsibleContent>
                </Collapsible>
              )}
            </>
          )}
        </DialogBody>
        {d && (
          <DialogFooter className="flex-wrap">
            {desktop && (
              <Button variant="quiet" disabled={exported.isPending} onClick={() => exported.mutate()}>
                导出这次整理的日志
              </Button>
            )}
            {exported.data && <span className="text-sm text-muted-foreground">已导出</span>}
            <ErrorText error={exported.error ?? remove.error} />
            <Confirm
              trigger={
                <Button variant="ghost" className="ml-auto text-destructive" disabled={remove.isPending}>
                  删除这次整理
                </Button>
              }
              message="删除这次整理？运行记录、读过的页面和尚未处理的候选会一起删除；已接受的导师不受影响。"
              action="删除"
              onConfirm={() => remove.mutate()}
            />
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
function Figure({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-1 font-medium break-words tabular-nums">{children}</dd>
    </div>
  );
}
/** "12,345 Token，约 0.12 元": the cost at the models' prices now, where they are filled in. */
function spent(t: TokenTotals): string {
  const costs = (["cny", "usd"] as const).flatMap((currency) => {
    const amount = t.estimatedCost[currency];
    return amount === null ? [] : [money(amount, currency, { about: true })];
  });
  return [`${(t.input + t.output).toLocaleString()} Token`, ...costs].join("，");
}
/** "1 小时 5 分钟": all the runs together, the one going on until now. */
function elapsed(runs: CrawlRun[], now: number): string {
  const ms = runs.reduce((sum, r) => sum + Math.max(0, (r.endedAt ?? now) - r.startedAt), 0);
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return "不到 1 分钟";
  const hours = Math.floor(minutes / 60);
  return hours > 0 ? `${hours} 小时 ${minutes % 60} 分钟` : `${minutes} 分钟`;
}
/**
 * docs/design.md#导师: the school and college the crawl found, written as the user corrects them,
 * which its candidates still to review follow.
 */
function CrawlUnit({ crawl: c, onEditing }: { crawl: CrawlSummary; onEditing: (editing: boolean) => void }) {
  const client = useQueryClient();
  const [form, setForm] = useState<{ university: string; school: string } | null>(null);
  const edit = (next: typeof form) => {
    setForm(next);
    onEditing(next !== null);
  };
  const save = useMutation({
    mutationFn: (unit: { university: string; school: string }) =>
      call("setCrawlUnit", { id: c.id, university: unit.university, school: unit.school.trim() || null }),
    onSuccess: () => {
      edit(null);
      refreshCrawls(client);
    },
  });
  if (!form) {
    const unit = c.university && (c.school ? `${c.university} ${c.school}` : `${c.university}（全校）`);
    return (
      <p className="flex flex-wrap items-center gap-x-2">
        {unit ? `学校和学院：${unit}` : <span className="text-muted-foreground">没认出学校和学院</span>}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => edit({ university: c.university ?? "", school: c.school ?? "" })}
        >
          {unit ? "修改" : "填写"}
        </Button>
      </p>
    );
  }
  return (
    <form
      className="mt-2 space-y-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate(form);
      }}
    >
      <div className="grid grid-cols-2 gap-3 phone:grid-cols-1">
        <Field label="学校" id="crawl-university">
          <Input
            id="crawl-university"
            value={form.university}
            onChange={(e) => setForm({ ...form, university: e.target.value })}
          />
        </Field>
        <Field label="学院（全校的名录不填）" id="crawl-school">
          <Input id="crawl-school" value={form.school} onChange={(e) => setForm({ ...form, school: e.target.value })} />
        </Field>
      </div>
      <p className="text-xs text-muted-foreground">
        这次整理还没审核的候选跟着改，写着别的学校、学院的不动；已接受的导师不改。
      </p>
      <ErrorText error={save.error} />
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" disabled={save.isPending} onClick={() => edit(null)}>
          取消
        </Button>
        <Button type="submit" disabled={!form.university.trim() || save.isPending}>
          保存
        </Button>
      </div>
    </form>
  );
}
const PAGE_KINDS: Record<PageKind, string> = {
  entry: "入口",
  list: "名单",
  profile: "个人页",
  enrich_profile: "补全",
  enrich_child: "补全",
};
function PageLine({ page: p, children }: { page: CrawlPage; children?: ReactNode }) {
  const failed = p.state === "done" && p.status !== "success";
  return (
    <div className="flex items-start gap-2 rounded-md bg-muted p-3">
      <div className="min-w-0 grow">
        <p className="break-all select-text">{p.url}</p>
        <p className={`mt-1 ${failed ? "text-warning" : "text-muted-foreground"}`}>
          {PAGE_KINDS[p.kind]} ·{" "}
          {p.state === "queued" ? "待读取" : p.state === "running" ? "正在读取" : (p.error?.message ?? "已读取")}
        </p>
        {failed && p.error && <FailureDetails failure={p.error} />}
      </div>
      {children}
    </div>
  );
}
/** Every page the crawl has read or is to read, fetched once opened. */
function AllPages({ id, total }: { id: number; total: number }) {
  const [open, setOpen] = useState(false);
  const [page, setPage] = useState(1);
  const pages = useQuery({
    queryKey: ["getCrawl", id, page, false],
    queryFn: () => call("getCrawl", { id, page, failed: false }),
    enabled: open,
    refetchInterval: 2000,
  });
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger>全部页面 · {total} 页</CollapsibleTrigger>
      <CollapsibleContent>
        <div className="space-y-2 pt-2">
          <ErrorText error={pages.error} />
          {pages.data?.pages.map((p) => (
            <PageLine key={p.id} page={p} />
          ))}
          <Pages page={page} total={pages.data?.pageTotal ?? 0} onPage={setPage} />
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
const COUNTS: Record<CandidateFilter, keyof CandidateCounts> = {
  all: "all",
  new: "new",
  supplement: "supplement",
  maybe: "maybe",
  missing_email: "missingEmail",
};
function Candidates() {
  const s = useSearch({ from: "/find" });
  const navigate = useNavigate({ from: "/find" });
  const client = useQueryClient();
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [enrich, setEnrich] = useState<number[] | null>(null);
  const [enrichResult, setEnrichResult] = useState<EnrichPlan | null>(null);
  const [review, setReview] = useState<number[] | null>(null);
  const [choices, setChoices] = useState<Record<number, Decisions>>({});
  const [error, setError] = useState<unknown>(null);
  const selection: CandidateSelection = {
    crawlId: s.crawl ?? null,
    filter: s.filter,
    processed: s.processed,
    keyword: s.keyword,
  };
  const q = { ...selection, page: s.page };
  // Ticks belong to one filter: changing it, including by going back, starts over; pages keep them.
  const criteria = JSON.stringify(selection);
  const [shownCriteria, setShownCriteria] = useState(criteria);
  if (shownCriteria !== criteria) {
    setShownCriteria(criteria);
    setSelected(new Set());
  }
  const list = useQuery({
    queryKey: ["listCandidates", q],
    queryFn: () => call("listCandidates", q),
    refetchInterval: 3000,
  });
  const scroller = useKeptScroll<HTMLDivElement>(list, JSON.stringify(q));
  const crawls = useQuery({
    queryKey: ["crawlOptions"],
    queryFn: () => call("crawlOptions", {}),
  });
  // docs/design.md: candidates from an Agent are marked, the app did not read their pages itself.
  const agentCrawls = new Set(crawls.data?.filter((c) => c.kind === "agent").map((c) => c.id));
  const items = list.data?.candidates ?? [];
  const total = list.data?.total ?? 0;
  const current = s.candidate ?? items[0]?.id;
  useEffect(() => {
    const compact = matchMedia(COMPACT);
    function openFirst() {
      if (!s.candidate && current !== undefined && !compact.matches)
        void navigate({ search: (old) => ({ ...old, candidate: current }), replace: true });
    }
    openFirst();
    compact.addEventListener("change", openFirst);
    return () => compact.removeEventListener("change", openFirst);
  }, [current, s.candidate, navigate]);
  /**
   * docs/ui.md#交互: a candidate opened over the list is a step that going back undoes; one opened
   * beside the list, or moved on to from the one before, is not.
   */
  function choose(candidate: number | undefined, step = matchMedia(COMPACT).matches) {
    void navigate({ search: (old) => ({ ...old, candidate }), replace: !step });
  }
  /** Shows another page of the list, and on it the candidate `pick` chooses. */
  function turn(page: number, pick: (shown: CandidatePage) => number | undefined) {
    const next = { ...q, page };
    void client
      .fetchQuery({ queryKey: ["listCandidates", next], queryFn: () => call("listCandidates", next) })
      .then((shown) => navigate({ search: (old) => ({ ...old, page, candidate: pick(shown) }), replace: true }))
      .catch(setError);
  }
  function move(delta: number) {
    const next = items[items.findIndex((c) => c.id === current) + delta];
    if (next) choose(next.id, false);
    else if (delta > 0 && s.page * 50 < total) turn(s.page + 1, (shown) => shown.candidates[0]?.id);
    else if (delta < 0 && s.page > 1) turn(s.page - 1, (shown) => shown.candidates.at(-1)?.id);
  }
  /** docs/ui.md#找导师: once one is reviewed, the next one is on show, which takes its place on the list. */
  function reviewed(id: number) {
    const index = items.findIndex((c) => c.id === id);
    choose((items[index + 1] ?? items[index - 1])?.id, false);
    refreshCrawls(client);
  }
  useEffect(() => {
    function key(e: KeyboardEvent) {
      const pressed = pageKey(e);
      if (pressed === "j" || pressed === "k") {
        e.preventDefault();
        move(pressed === "j" ? 1 : -1);
      }
    }
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  });
  const tickAll = useMutation({
    mutationFn: (asked: { selection: CandidateSelection; from: ReadonlySet<number> }) =>
      call("listCandidateIds", asked.selection),
    // Only over the ticks it was asked from: ticking or unticking while it ran replaced them.
    onSuccess: (ids, { from }) => setSelected((old) => (old === from ? new Set(ids) : old)),
  });
  const reject = useMutation({
    mutationFn: (ids: number[]) => call("rejectCandidates", { ids }),
    // Only these: others may have been ticked while it ran.
    onSuccess: (_, ids) => {
      const gone = new Set(ids);
      setSelected((old) => new Set([...old].filter((id) => !gone.has(id))));
      refreshCrawls(client);
    },
  });
  const change = (part: Partial<typeof s>) => {
    void navigate({ search: (old) => ({ ...old, ...part, page: 1, candidate: undefined }), replace: true });
  };
  const tick = (id: number, ticked: boolean) =>
    setSelected((old) => {
      const next = new Set(old);
      if (ticked) next.add(id);
      else next.delete(id);
      return next;
    });
  const chosen = s.candidate !== undefined;
  const all = total > 0 && selected.size === total;
  return (
    <>
      <div className={`shrink-0 space-y-3 px-6 pb-3 phone:px-4 ${chosen ? "compact:hidden" : ""}`}>
        <div className="flex flex-wrap items-center gap-2">
          <Segments
            label="候选分类"
            value={s.filter}
            options={Object.entries(FILTERS).map(([value, label]) => {
              const filter = findSearch({ filter: value }).filter;
              return [
                filter,
                <>
                  {label}
                  {list.data && (
                    <span className="text-xs text-faint tabular-nums">{list.data.counts[COUNTS[filter]]}</span>
                  )}
                </>,
              ] as const;
            })}
            onChange={(filter) => change({ filter })}
          />
          <div className="grow" />
          <SearchBox
            label="搜索候选"
            placeholder="搜索姓名、单位、研究方向或邮箱"
            className="w-80 phone:order-last phone:w-full"
            value={s.keyword ?? ""}
            onChange={(keyword) => change({ keyword: keyword || undefined })}
          />
          <PillSelect
            label="整理记录"
            options={[
              [undefined, "不限"],
              ...(crawls.data ?? []).map(
                (c) => [c.id, `${day(c.createdAt)} ${time(c.createdAt)} · ${crawlName(c)}`] as const,
              ),
            ]}
            value={s.crawl}
            active={s.crawl !== undefined}
            onChange={(crawl) => change({ crawl })}
          />
          <Button
            variant={s.processed ? "quiet" : "ghost"}
            aria-pressed={s.processed}
            onClick={() => change({ processed: !s.processed })}
          >
            {s.processed ? "返回待审核" : "已处理"}
          </Button>
        </div>
        {enrichResult && (
          <p role="status" className="text-sm text-muted-foreground">
            已安排补全 {enrichResult.ids.length} 位<SkippedEnrich plan={enrichResult} />
          </p>
        )}
        <ErrorText error={list.error ?? crawls.error ?? error} />
      </div>
      {list.isPending ? (
        <Loading />
      ) : total === 0 ? (
        <div className="min-h-0 flex-1 overflow-y-auto px-6 phone:px-4">
          {s.filter !== "all" || s.crawl !== undefined || s.keyword !== undefined ? (
            <Notice icon={SearchX} title="无符合条件的候选" detail="可更换筛选条件查看" />
          ) : s.processed ? (
            <Notice icon={Inbox} title="尚无已处理的候选" detail="接受或拒绝后的候选会列在这里" />
          ) : (
            <Notice
              icon={Inbox}
              title="尚无候选"
              detail="智能整理找到的导师先列在这里，审核后加入导师列表"
              actions={<Button onClick={() => change({ tab: "crawl" })}>开始智能整理</Button>}
            />
          )}
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(280px,360px)_minmax(0,1fr)] border-t border-border compact:grid-cols-1">
          <section
            className={`relative flex min-h-0 flex-col border-r border-border compact:border-0 ${chosen ? "compact:hidden" : ""}`}
          >
            {!s.processed && (
              <label className="flex min-h-11 shrink-0 cursor-pointer items-center gap-2 px-5 text-sm">
                <Checkbox
                  checked={all}
                  indeterminate={selected.size > 0 && !all}
                  disabled={tickAll.isPending}
                  onCheckedChange={(checked) =>
                    checked ? tickAll.mutate({ selection, from: selected }) : setSelected(new Set())
                  }
                />
                全选筛出的 {total.toLocaleString()} 位
              </label>
            )}
            <div ref={scroller} className="min-h-0 flex-1 space-y-[3px] overflow-y-auto px-2 pb-24">
              {items.map((c) => (
                <CandidateLine
                  key={c.id}
                  candidate={c}
                  current={c.id === current}
                  agent={c.crawlId !== null && agentCrawls.has(c.crawlId)}
                  ticked={s.processed ? null : selected.has(c.id)}
                  onTick={(ticked) => tick(c.id, ticked)}
                  onChoose={() => choose(c.id)}
                />
              ))}
              <Pages
                page={s.page}
                total={total}
                onPage={(page) =>
                  void navigate({ search: (old) => ({ ...old, page, candidate: undefined }), replace: true })
                }
              />
            </div>
            {!s.processed && (
              <BatchBar
                count={selected.size}
                errors={[
                  ...(tickAll.error ? [{ lead: "未能全选：", failure: failureOf(tickAll.error) }] : []),
                  ...(reject.error ? [{ lead: "未能拒绝：", failure: failureOf(reject.error) }] : []),
                ]}
                onClear={() => {
                  setSelected(new Set());
                  for (const action of [tickAll, reject]) if (!action.isPending) action.reset();
                }}
              >
                <Button size="sm" onClick={() => setReview([...selected])}>
                  接受
                </Button>
                <Button size="sm" variant="quiet" onClick={() => setEnrich([...selected])}>
                  补全
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={reject.isPending}
                  onClick={() => reject.mutate([...selected])}
                >
                  拒绝
                </Button>
              </BatchBar>
            )}
          </section>
          {current === undefined ? (
            <section className="compact:hidden" />
          ) : (
            <CandidatePane
              key={current}
              id={current}
              row={items.find((c) => c.id === current)}
              agent={items.some((c) => c.id === current && c.crawlId !== null && agentCrawls.has(c.crawlId))}
              chosen={chosen}
              decisions={choices[current] ?? EMPTY}
              onChange={(next) =>
                setChoices((previous) => ({
                  ...previous,
                  [current]: typeof next === "function" ? next(previous[current] ?? EMPTY) : next,
                }))
              }
              onBack={() => choose(undefined, false)}
              onMove={move}
              onReviewed={() => reviewed(current)}
              onEnrich={() => setEnrich([current])}
            />
          )}
        </div>
      )}
      {enrich && (
        <EnrichCandidatesDialog
          ids={enrich}
          onClose={() => setEnrich(null)}
          onDone={(result) => {
            setEnrichResult(result);
            setEnrich(null);
            setSelected(new Set());
            refreshCrawls(client);
          }}
        />
      )}
      {review && (
        <ReviewDialog
          ids={review}
          initialDecisions={review.length === 1 && review[0] !== undefined ? (choices[review[0]] ?? EMPTY) : EMPTY}
          onClose={() => setReview(null)}
          onDone={() => {
            setSelected(new Set());
            setReview(null);
            refreshCrawls(client);
            refreshMentors(client);
          }}
        />
      )}
    </>
  );
}
/** docs/ui.md#找导师: what to look out for in a candidate, written at the end of their line. */
function flags(c: CandidateRow, agent: boolean): [string, string][] {
  const all: [boolean, string, string][] = [
    [c.category === "supplement", "可补充", "text-brand-text"],
    [c.category === "maybe", "可能是同一位", "text-warning"],
    [c.recognized.length > 0, "从图片识别", "text-warning"],
    [agent, "Agent 提交", "text-muted-foreground"],
    [c.enrich === "queued" || c.enrich === "running", "补全中", "text-muted-foreground"],
  ];
  return all.filter(([on]) => on).map(([, label, tone]) => [label, tone]);
}
function CandidateLine({
  candidate: c,
  current,
  agent,
  ticked,
  onTick,
  onChoose,
}: {
  candidate: CandidateRow;
  current: boolean;
  agent: boolean;
  /** None among those already reviewed, which are not ticked. */
  ticked: boolean | null;
  onTick: (ticked: boolean) => void;
  onChoose: () => void;
}) {
  const marks: [string, string][] =
    ticked === null ? [[c.review === "rejected" ? "已拒绝" : "已接受", "text-muted-foreground"]] : flags(c, agent);
  return (
    <div className={`flex animate-fade-in items-start rounded-md ${current ? "bg-selected" : "hover:bg-muted"}`}>
      {ticked !== null && (
        <label className="flex min-h-[58px] min-w-10 shrink-0 cursor-pointer items-center justify-center">
          <Checkbox aria-label={`选择${c.name}`} checked={ticked} onCheckedChange={onTick} />
        </label>
      )}
      <button
        className={`min-w-0 grow py-2.5 pr-3 text-left ${ticked === null ? "pl-3" : ""}`}
        aria-current={current}
        onClick={onChoose}
      >
        <span className="flex items-baseline gap-2">
          <span className="truncate font-medium">{c.name}</span>
          {c.affiliation?.title && (
            <span className="min-w-0 truncate text-xs text-muted-foreground">{c.affiliation.title}</span>
          )}
          <span className="grow" />
          {marks.map(([label, tone]) => (
            <span key={label} className={`shrink-0 text-xs ${tone}`}>
              {label}
            </span>
          ))}
        </span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground">
          {c.emails.length > 0 ? c.emails.join("、") : <span className="text-warning">缺邮箱</span>}
        </span>
      </button>
    </div>
  );
}
const ENRICH_FIELDS: Record<EnrichField, string> = {
  email: "邮箱",
  alias: "别名",
  homepage: "主页",
  paper: "代表论文",
  research: "研究方向",
  affiliation: "单位",
  university: "学校",
  school: "学院",
  department: "系所",
  title: "职称",
};
const ENRICH_SKIP: Record<EnrichSkipReason, string> = {
  missing_profile: "无个人页",
  deleted_crawl: "原整理记录已删除",
  complete: "资料已全",
  active: "正在补全",
  reviewed: "已处理或已删除",
  trashed: "在回收站里",
  pending: "候选里有待审核的",
  shared: "和另一位是同一个页面",
};
function SkippedEnrich({ plan }: { plan: EnrichPlan }) {
  const count = plan.skipped.reduce((sum, group) => sum + group.ids.length, 0);
  return count > 0 ? (
    <>
      {" "}
      · 跳过 {count} 位（{plan.skipped.map((group) => `${ENRICH_SKIP[group.reason]} ${group.ids.length} 位`).join("、")}
      ）
    </>
  ) : null;
}
function EnrichCandidatesDialog({
  ids,
  onClose,
  onDone,
}: {
  ids: number[];
  onClose: () => void;
  onDone: (result: EnrichPlan) => void;
}) {
  const preview = useQuery({ queryKey: ["previewEnrich", ids], queryFn: () => call("previewEnrich", { ids }) });
  const start = useMutation({ mutationFn: () => call("enrichCandidates", { ids }), onSuccess: onDone });
  return (
    <EnrichDialog
      title="补全候选信息"
      preview={preview}
      start={start}
      note="只补空着的资料，保留已有值。网页内容和候选姓名、别名、已有单位将发给模型服务商。可在后台暂停或取消；所属整理已暂停时，补全也会等待继续。"
      onClose={onClose}
    />
  );
}
/** docs/design.md#导师: what is found for mentors already in the list waits as candidates for review. */
export function EnrichMentorsDialog({ ids, onClose }: { ids: number[]; onClose: () => void }) {
  const client = useQueryClient();
  const preview = useQuery({
    queryKey: ["previewEnrichMentors", ids],
    queryFn: () => call("previewEnrichMentors", { ids }),
  });
  const start = useMutation({
    mutationFn: () => call("enrichMentors", { ids }),
    onSuccess: () => refreshCrawls(client),
  });
  const started = start.data;
  return (
    <EnrichDialog
      title="补全导师资料"
      preview={preview}
      start={start}
      note="只补空着的资料，补到的先放进候选，审核后才并进导师。网页内容和导师姓名、别名、已有单位将发给模型服务商，个人备注和标签不会发送。可在后台暂停或取消。"
      onClose={onClose}
    >
      {started && (
        <p role="status">
          {started.crawlId === null ? "没有需要补全的导师" : `已开始补全 ${started.ids.length} 位`}
          <SkippedEnrich plan={started} />
          {started.crawlId !== null && (
            <Link
              to="/find"
              search={findSearch({ crawl: started.crawlId })}
              className="ml-2 text-brand-text"
              onClick={onClose}
            >
              去审核
            </Link>
          )}
        </p>
      )}
    </EnrichDialog>
  );
}

/** "约 0.12～0.48 元": each mentor reads one to four pages. */
function costRange(rate: ModelRate, mentors: number) {
  const least = costAt(rate, PAGE.input * mentors, PAGE.output * mentors);
  const most = least * 4;
  const unit = UNITS[rate.currency];
  // As in money(), what costs less than 0.01 never reads 0.00: then only the most it may come to.
  let range = `约 ${least.toFixed(2)}～${most.toFixed(2)} ${unit}`;
  if (least > 0 && least < 0.01) range = most < 0.01 ? money(most, rate.currency) : `最多约 ${most.toFixed(2)} ${unit}`;
  return range + hoursNote(rate);
}

/** Once started, `children` tells how it went in place of the plan, which then no longer holds. */
function EnrichDialog({
  title,
  preview,
  start,
  note,
  onClose,
  children,
}: {
  title: string;
  preview: UseQueryResult<EnrichPlan>;
  start: UseMutationResult<EnrichPlan, Error, void>;
  note: string;
  onClose: () => void;
  children?: ReactNode;
}) {
  const client = useQueryClient();
  const models = useQuery({ queryKey: ["listModelConfigs"], queryFn: () => call("listModelConfigs", {}) });
  const chosen = models.data && featureModel(models.data, "crawl");
  const count = preview.data?.ids.length ?? 0;
  const rate = chosen?.rate;
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !start.isPending) onClose();
      }}
    >
      <DialogContent className="w-[520px]">
        <DialogHeader>{title}</DialogHeader>
        <DialogBody className="space-y-4 text-sm">
          {children ?? (
            <>
              {preview.data && (
                <p>
                  补全 {count} 位<SkippedEnrich plan={preview.data} />
                </p>
              )}
              <p>模型：{chosen ? `${chosen.name || chosen.model}（${chosen.model}）` : "尚未添加模型"}</p>
              <p className="text-muted-foreground">
                每位预计读取 1～4 页，约 {PAGE_TOKENS}～{PAGE_TOKENS * 4} Token；本次约{" "}
                {(count * PAGE_TOKENS).toLocaleString()}～{(count * PAGE_TOKENS * 4).toLocaleString()} Token
                {rate && `，${costRange(rate, count)}`}
                。仅为估算，以实际页面和服务商报告为准。
              </p>
            </>
          )}
          <p className="text-muted-foreground">{note}</p>
          <ErrorText error={preview.error ?? models.error ?? start.error} />
        </DialogBody>
        <DialogFooter>
          <Button variant="ghost" disabled={start.isPending} onClick={onClose}>
            关闭
          </Button>
          <ModelAction
            feature="crawl"
            disabled={!preview.data || count === 0 || start.isPending || start.isSuccess}
            onWrite={() =>
              start.mutate(undefined, { onError: () => client.invalidateQueries({ queryKey: ["listModelConfigs"] }) })
            }
          >
            {start.isPending ? "正在安排…" : "开始补全"}
          </ModelAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
/** docs/ui.md#找导师: one candidate beside the list, or over it in a narrow window, to accept or reject. */
function CandidatePane({
  id,
  row,
  agent,
  chosen,
  decisions,
  onChange,
  onBack,
  onMove,
  onReviewed,
  onEnrich,
}: {
  id: number;
  /** Their line on the list, which tells what kind of candidate they are. */
  row: CandidateRow | undefined;
  agent: boolean;
  /** Whether the user chose them, which a narrow window then shows in place of the list. */
  chosen: boolean;
  decisions: Decisions;
  onChange: (next: Decisions | ((previous: Decisions) => Decisions)) => void;
  onBack: () => void;
  onMove: (delta: number) => void;
  onReviewed: () => void;
  onEnrich: () => void;
}) {
  const client = useQueryClient();
  const detail = useQuery({
    queryKey: ["getCandidate", id],
    queryFn: () => call("getCandidate", { id }),
    // What enrichment finds fills them in meanwhile.
    refetchInterval: (query) =>
      query.state.data?.enrich === "queued" || query.state.data?.enrich === "running" ? 3000 : false,
  });
  const d = detail.data;
  const pending = d?.review === "pending";
  const preview = useQuery({
    queryKey: ["previewCandidates", [id], decisions],
    queryFn: () => call("previewCandidates", { ids: [id], decisions }),
    enabled: pending,
  });
  const accept = useMutation({
    mutationFn: () => call("acceptCandidates", { ids: [id], decisions }),
    onSuccess: (r) => {
      refreshMentors(client);
      if (r.accepted > 0) onReviewed();
      else void preview.refetch();
    },
  });
  const reject = useMutation({ mutationFn: () => call("rejectCandidates", { ids: [id] }), onSuccess: onReviewed });
  const pane = `flex min-h-0 min-w-0 flex-col ${chosen ? "" : "compact:hidden"}`;
  const back = <BackButton up={onBack} className="-ml-2 hidden compact:inline-flex" />;
  if (!d)
    return (
      <section className={pane}>
        <div className="px-6 pt-4 phone:px-3">{back}</div>
        {detail.error ? <Notice title="候选未能加载" failure={failureOf(detail.error)} /> : <Loading />}
      </section>
    );
  const f = d.fields;
  const issue = row?.issue ?? preview.data?.errors[0]?.reason;
  const kind = pending
    ? row && row.category !== "all" && row.category !== "missing_email" && FILTERS[row.category]
    : d.review === "rejected"
      ? "已拒绝"
      : "已接受";
  return (
    <section className={pane}>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <header className="flex items-start gap-1 px-6 pt-5 phone:px-3">
          {back}
          <div className="min-w-0 grow">
            <h2 className="text-xl font-semibold break-words select-text">{f.name}</h2>
            <p className="mt-1 text-sm text-muted-foreground empty:hidden">
              {[kind, agent && "Agent 提交", f.aliases.length > 0 && `又名 ${f.aliases.join("、")}`]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>
        </header>
        <div className="space-y-5 px-6 py-4 text-sm phone:px-3">
          {d.note && <p className="rounded-md bg-warning-soft p-3 break-words text-warning">Agent 说明：{d.note}</p>}
          <dl className="grid grid-cols-[4.5rem_minmax(0,1fr)] gap-x-4 gap-y-3 select-text">
            <Fact label="单位">
              {f.affiliations.length > 0 ? (
                f.affiliations.map((a, n) => <p key={n}>{[placeOf(a), a.title].filter(Boolean).join(" · ")}</p>)
              ) : (
                <Missing>未填写</Missing>
              )}
            </Fact>
            <Fact label="邮箱">
              {f.emails.length > 0 ? (
                f.emails.map((m) => <p key={m}>{m}</p>)
              ) : (
                <span className="text-warning">缺邮箱</span>
              )}
            </Fact>
            <Fact label="研究方向">
              {f.research ? <p className="whitespace-pre-wrap">{f.research}</p> : <Missing>未填写</Missing>}
            </Fact>
            {f.homepages.length > 0 && (
              <Fact label="主页">
                {f.homepages.map((url) => (
                  <WebLink key={url} url={url} />
                ))}
              </Fact>
            )}
            {d.papers.length > 0 && (
              <Fact label="代表论文">
                {d.papers.map((p, n) => (
                  <p key={n}>{p}</p>
                ))}
              </Fact>
            )}
            {d.sourceUrl && (
              <Fact label="来源页">
                <WebLink url={d.sourceUrl} />
              </Fact>
            )}
          </dl>
          {d.recognized.length > 0 && (
            <p className="text-warning">{d.recognized.join("、")} 是从图片识别的，可能认错字母，请打开来源页核对</p>
          )}
          {(d.enrich === "queued" || d.enrich === "running") && (
            <p role="status" className="text-muted-foreground">
              正在补全…
            </p>
          )}
          {d.enrichChanges.length > 0 && (
            <p className="text-brand-text">
              补到了{[...new Set(d.enrichChanges.map((v) => ENRICH_FIELDS[v.field]))].join("、")}
            </p>
          )}
          {d.enrichError && <FailureNote failure={d.enrichError} lead="没补到：" className="text-warning" />}
          {pending && preview.data && (
            <IntakeChoices
              preview={preview.data}
              decisions={decisions}
              onChange={onChange}
              source="候选"
              numbered={false}
            />
          )}
          {pending && issue && <p className="text-warning">{issue}</p>}
          {accept.data?.accepted === 0 && (
            <p role="status" className="text-warning">
              {preview.data?.trashed.length
                ? "回收站中的导师默认跳过，选择“恢复并更新”后才能接受"
                : "未能接受，请查看上面的原因"}
            </p>
          )}
          <Collapsible>
            <CollapsibleTrigger>来源与邮箱证据 · {d.evidence.length} 页</CollapsibleTrigger>
            <CollapsibleContent>
              {d.evidence.map((e, n) => (
                <div key={n} className="mt-2 break-words text-muted-foreground">
                  <WebLink url={e.url} className="text-xs" />
                  <p>
                    {e.name} ·{" "}
                    {e.emails.map((m) => (e.recognized.includes(m) ? `${m}（从图片识别）` : m)).join("、") ||
                      "未找到可确认归属的邮箱"}
                  </p>
                  {e.research && <p>{e.research}</p>}
                </div>
              ))}
            </CollapsibleContent>
          </Collapsible>
        </div>
      </div>
      <footer className="flex flex-wrap items-center gap-2 border-t border-border px-6 py-3 phone:px-3">
        {pending ? (
          <>
            <Button disabled={!!issue || !preview.data || accept.isPending} onClick={() => accept.mutate()}>
              {row?.category === "supplement" ? "补充" : "接受"}
            </Button>
            <Button variant="ghost" disabled={reject.isPending} onClick={() => reject.mutate()}>
              拒绝
            </Button>
            {(f.emails.length === 0 || !f.research) && (
              <Button variant="quiet" onClick={onEnrich}>
                补全
              </Button>
            )}
          </>
        ) : (
          d.review !== "rejected" &&
          d.mentorId !== null && (
            <Link to="/mentors/$mentorId" params={{ mentorId: d.mentorId }} className="text-sm text-brand-text">
              查看导师
            </Link>
          )
        )}
        <span className="grow" />
        <Button variant="ghost" className="hidden compact:inline-flex" onClick={() => onMove(-1)}>
          上一位
        </Button>
        <Button variant="ghost" className="hidden compact:inline-flex" onClick={() => onMove(1)}>
          下一位
        </Button>
        <ErrorText error={accept.error ?? reject.error} />
      </footer>
    </section>
  );
}
function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 space-y-1 break-words">{children}</dd>
    </>
  );
}
function ReviewDialog({
  ids,
  initialDecisions,
  onClose,
  onDone,
}: {
  ids: number[];
  initialDecisions: Decisions;
  onClose: () => void;
  onDone: () => void;
}) {
  const [decisions, setDecisions] = useState<Decisions>(initialDecisions);
  const preview = useQuery({
    queryKey: ["previewCandidates", ids, decisions],
    queryFn: () => call("previewCandidates", { ids, decisions }),
  });
  const accept = useMutation({
    mutationFn: () => call("acceptCandidates", { ids, decisions }),
    onSuccess: (r) => {
      if (r.skipped > 0) {
        void preview.refetch();
        setResult(`已接受 ${r.accepted} 位，另有 ${r.skipped} 位尚未接受，请查看原因`);
      } else {
        onDone();
      }
    },
  });
  const [result, setResult] = useState("");
  return (
    <Dialog
      open
      onOpenChange={(open, event) => {
        if (!open && !accept.isPending && event.reason !== "outside-press" && event.reason !== "escape-key") onClose();
      }}
    >
      <DialogContent className="w-[920px]">
        <DialogHeader>接受候选 · {ids.length} 位</DialogHeader>
        <DialogBody className="space-y-4">
          <p className="text-sm text-muted-foreground">
            只补充资料，空项不会清除本地内容。资料不同的默认保留本地，缺邮箱也可接受。
          </p>
          {preview.data && (
            <>
              <p className="rounded-md bg-muted p-3 text-sm">
                新建 {preview.data.created} 位 · 更新 {preview.data.updated} 位 · 无需修改 {preview.data.unchanged} 位
              </p>
              <IntakeChoices preview={preview.data} decisions={decisions} onChange={setDecisions} source="候选" />
              {preview.data.errors.map((e) => (
                <p key={e.row} className="text-sm text-warning">
                  第 {e.row} 位：{e.reason}
                </p>
              ))}
              {preview.data.trashed.length > 0 && (
                <p className="text-sm text-warning">回收站中的导师默认跳过，选择“恢复并更新”后才能接受</p>
              )}
            </>
          )}
          {result && (
            <p role="status" className="text-sm text-warning">
              {result}
            </p>
          )}
          <ErrorText error={preview.error ?? accept.error} />
        </DialogBody>
        <DialogFooter>
          <Button variant="ghost" disabled={accept.isPending} onClick={onClose}>
            关闭
          </Button>
          <Button
            disabled={
              !preview.data || preview.isFetching || accept.isPending || ids.length === preview.data.errors.length
            }
            onClick={() => accept.mutate()}
          >
            {accept.isPending ? "正在接受…" : "接受所选候选"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
function ErrorText({ error }: { error: unknown }) {
  return error ? <FailureNote className="text-sm text-destructive" role="alert" failure={failureOf(error)} /> : null;
}
function WebLink({ url, className }: { url: string; className?: string }) {
  return (
    <a
      href={url}
      className={cn("block break-all text-brand-text hover:underline", className)}
      onClick={(e) => {
        e.preventDefault();
        void openUrl(url);
      }}
    >
      {url}
    </a>
  );
}
function Pages(props: { page: number; total: number; onPage: (page: number) => void }) {
  return (
    <div className="mt-3 flex justify-end empty:hidden">
      <Pager {...props} />
    </div>
  );
}
