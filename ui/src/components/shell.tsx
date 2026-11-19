import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, Outlet, useLocation, useRouter, useRouterState } from "@tanstack/react-router";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  ChartNoAxesColumn,
  CircleEllipsis,
  ListTodo,
  LoaderCircle,
  MessagesSquare,
  PanelLeft,
  Search,
  Settings,
  Users,
  X,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "cn";
import appIcon from "../../../docs/icon.svg";
import { call, errorMessage } from "@/api/call";
import { startupFailure } from "@/api/app";
import type { MatchFinished, DraftsFinished, CrawlFinished } from "@/api/types";
import { refreshTasks } from "@/components/task-cache";
import { refreshMentors, refreshReplyOutcomes } from "@/components/mentor";
import { refreshTodos } from "@/components/todos";
import { Button } from "@/components/ui/button";
import { Tooltip } from "@/components/ui/tooltip";
import { StartupFailure } from "@/components/startup-failure";
import { showsFeedbackLog } from "@/components/feedback";
import { AgentRequests } from "@/components/agent-requests";
import { TermsUpdate, useAccount } from "@/components/account";
import { UpdateEntry } from "@/components/update";
import { LayerSteps, openSection } from "@/back";
import { desktop, dragsWindow, titleBar } from "@/host";
import { useBackground } from "@/pages/background";
import { findSearch, refreshCrawls } from "@/pages/find";

const NAV = [
  { to: "/mentors", label: "导师", icon: Users },
  { to: "/conversations", label: "往来", icon: MessagesSquare },
  { to: "/tasks", label: "任务", icon: ListTodo },
  { to: "/find", label: "找导师", icon: Search },
  { to: "/stats", label: "统计", icon: ChartNoAxesColumn },
] as const;

const MORE = { to: "/more", label: "更多", icon: CircleEllipsis } as const;
// docs/ui.md#布局: settings open from the account row at the bottom of the sidebar.
const SETTINGS = { to: "/settings", label: "设置", icon: Settings } as const;

/**
 * Sidebar on the window background, content on a rounded page; on a phone-width window the
 * sidebar becomes a tab bar under the page (docs/ui.md#布局).
 */
const focusSearch = () => document.getElementById("mentor-search")?.focus();

/** Where each section of the sidebar was left in this run, to return to (docs/ui.md#交互). */
const left = new Map<string, string>();

/**
 * Remembers where the sections are left, and makes the click on a section's entry go back there,
 * scrolled as it was, in place of all opened before it (docs/ui.md#交互). On the section itself the
 * entry starts it afresh, as it does when not yet visited; with a key held down it opens as any
 * link does.
 */
function useOpenSection() {
  const router = useRouter();
  const pathname = useLocation({ select: (l) => l.pathname });
  const shown = useRouterState({ select: (s) => s.resolvedLocation?.href });
  useEffect(() => {
    const section = NAV.find((n) => n.to === shown?.split("?")[0])?.to;
    if (shown && section) left.set(section, shown);
  }, [shown]);
  return (to: string) => (e: React.MouseEvent) => {
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    const href = pathname === to ? undefined : left.get(to);
    void openSection(router, href ?? to, href !== undefined);
  };
}
// macOS keeps Ctrl+K for deleting to the end of the line; Windows takes Win+K for itself.
const SEARCH_KEY = navigator.userAgent.includes("Mac") ? "metaKey" : "ctrlKey";

export function Shell() {
  const failure = useQuery({ queryKey: ["startupFailure"], queryFn: startupFailure, staleTime: Infinity });
  const queryClient = useQueryClient();
  const [completed, setCompleted] = useState<string | null>(null);
  const router = useRouter();
  const pathname = useLocation({ select: (l) => l.pathname });
  // docs/ui.md#布局: kept on this computer for the next start.
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem("sidebar") === "collapsed");
  const collapse = (to: boolean) => {
    if (to) localStorage.setItem("sidebar", "collapsed");
    else localStorage.removeItem("sidebar");
    setCollapsed(to);
  };
  // docs/ui.md#快捷键: ⌘K (Ctrl+K on Windows) searches mentors from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e[SEARCH_KEY] || e.key.toLowerCase() !== "k" || e.defaultPrevented) return;
      // Leaving the page would close the dialog and lose what was typed in it.
      if (document.querySelector('[role="dialog"]')) return;
      e.preventDefault();
      if (pathname === "/mentors") focusSearch();
      else void openSection(router, "/mentors").then(focusSearch);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [router, pathname]);
  useEffect(() => {
    const crawlChanged = listen("crawl-changed", () => refreshCrawls(queryClient));
    const crawlFinished = listen<CrawlFinished>("crawl-finished", ({ payload: p }) => {
      refreshCrawls(queryClient);
      setCompleted(p.message);
    });
    const stop = listen("matching-changed", () => refreshMentors(queryClient));
    const done = listen<MatchFinished>("matching-finished", ({ payload: p }) => {
      refreshMentors(queryClient);
      setCompleted(`匹配度分析完成：分析了 ${p.analyzed} 位${p.failed > 0 ? `，失败 ${p.failed} 位` : ""}`);
    });
    // A sent email changes its mentor's correspondence and contact status as well as its task.
    const sendingChanged = listen("sending-changed", () => {
      refreshMentors(queryClient);
      // The official server's pauses come back with what it sent.
      void queryClient.invalidateQueries({ queryKey: ["listMailboxes"] });
    });
    // Opens and registrations show on letters, in the contact column and in the filters.
    const trackingChanged = listen("tracking-changed", () => {
      refreshMentors(queryClient);
      void queryClient.invalidateQueries({ queryKey: ["getTracking"] });
    });
    const accountChanged = listen("account-changed", () => queryClient.invalidateQueries({ queryKey: ["getAccount"] }));
    const tasksChanged = listen("tasks-changed", () => refreshTasks(queryClient));
    const repliesChanged = listen("reply-processing-changed", () => refreshReplyOutcomes(queryClient));
    const todosChanged = listen("todos-changed", () => refreshTodos(queryClient));
    const updateChanged = listen("update-changed", () => queryClient.invalidateQueries({ queryKey: ["updateStatus"] }));
    const draftsDone = listen<DraftsFinished>("drafts-finished", ({ payload: p }) => {
      refreshTasks(queryClient);
      setCompleted(`任务“${p.name}”的草稿已生成：${p.generated} 封，失败 ${p.failed} 封`);
    });
    return () => {
      void crawlChanged.then((f) => f());
      void crawlFinished.then((f) => f());
      void sendingChanged.then((f) => f());
      void trackingChanged.then((f) => f());
      void accountChanged.then((f) => f());
      void tasksChanged.then((f) => f());
      void repliesChanged.then((f) => f());
      void todosChanged.then((f) => f());
      void updateChanged.then((f) => f());
      void draftsDone.then((f) => f());
      void stop.then((f) => f());
      void done.then((f) => f());
    };
  }, [queryClient]);
  useEffect(() => {
    if (!completed) return undefined;
    const timer = setTimeout(() => setCompleted(null), 6000);
    return () => clearTimeout(timer);
  }, [completed]);
  // New mail may change any mentor's correspondence and contact status, and a mailbox may have
  // failed to receive or recovered.
  useEffect(() => {
    const stop = listen("received", () => {
      refreshMentors(queryClient);
      for (const key of ["listMailboxes", "receiveStatus"]) {
        void queryClient.invalidateQueries({ queryKey: [key] });
      }
    });
    return () => void stop.then((unlisten) => unlisten());
  }, [queryClient]);
  // What an Agent or a phone did may touch anything on screen, but for the feedback dialog's log,
  // which stays the one it will send. An Agent often calls many operations in a row, so refresh at
  // most twice a second while it works.
  useEffect(() => {
    let waiting = false;
    const stop = listen("called-elsewhere", () => {
      if (waiting) return;
      waiting = true;
      setTimeout(() => {
        waiting = false;
        void queryClient.invalidateQueries({ predicate: (query) => !showsFeedbackLog(query) });
      }, 500);
    });
    return () => void stop.then((unlisten) => unlisten());
  }, [queryClient]);
  // docs/ui.md#动效: another page fades in.
  const page = useRouterState({ select: (s) => s.matches[1]?.routeId });
  const toSection = useOpenSection();
  if (failure.isPending) return null;
  if (failure.data) return <StartupFailure failure={failure.data} />;
  return (
    <div className="relative flex h-dvh phone:flex-col phone:pt-(--title-bar)" {...dragsWindow()}>
      <SidebarToggle collapsed={collapsed} onClick={() => collapse(!collapsed)} />
      <aside
        className={cn(
          "flex shrink-0 flex-col overflow-hidden px-3 pb-4 transition-[width] phone:hidden",
          // Below the macOS window's buttons; elsewhere below the toggle.
          titleBar > 0 ? "pt-[calc(var(--title-bar)+16px)]" : "pt-16",
          // docs/ui.md#动效: what goes away goes faster.
          collapsed ? "w-[66px] duration-160 ease-in" : "w-[228px] duration-240 ease-out",
        )}
        {...dragsWindow("deep")}
      >
        {/* At the width it ends at: as the sidebar widens, the names are uncovered, not squeezed. */}
        <div className={cn("flex min-h-0 flex-1 flex-col", collapsed ? "w-[42px]" : "w-[204px]")}>
          <div className="flex items-center gap-2.5 px-1.5 pb-5 font-semibold">
            <img src={appIcon} alt="" className="size-[30px] shrink-0" />
            <span className={collapsed ? "sr-only" : "whitespace-nowrap"}>Auto Email Sender</span>
          </div>
          <nav className="flex flex-col gap-0.5">
            {NAV.map((item) => (
              <NavLink
                key={item.to}
                {...item}
                collapsed={collapsed}
                onClick={toSection(item.to)}
                count={item.to === "/conversations" ? "unread" : item.to === "/tasks" ? "review" : undefined}
              />
            ))}
          </nav>
          <div className="grow" />
          <div className="flex flex-col gap-0.5">
            {desktop && <UpdateEntry collapsed={collapsed} />}
            <BackgroundEntry collapsed={collapsed} onClick={toSection("/background")} />
            <AccountRow collapsed={collapsed} onClick={toSection(SETTINGS.to)} />
          </div>
        </div>
      </aside>
      <main className="relative mt-[max(var(--title-bar),16px)] mr-4 mb-4 flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-background phone:m-0 phone:min-h-0 phone:rounded-none">
        <div key={page} className="flex min-h-0 flex-1 animate-enter flex-col">
          <Outlet />
        </div>
        <LayerSteps />
        {desktop && <AgentRequests />}
        {desktop && <TermsUpdate />}
        {completed && (
          <div
            key={completed}
            role="status"
            className="absolute bottom-5 left-5 z-40 flex max-w-[calc(100%-40px)] animate-enter items-center gap-2 rounded-lg border border-border bg-background py-2 pr-2 pl-4 text-sm shadow-pop phone:bottom-3 phone:left-3"
          >
            <span>{completed}</span>
            <Button variant="ghost" size="icon" aria-label="关闭提醒" onClick={() => setCompleted(null)}>
              <X />
            </Button>
          </div>
        )}
      </main>
      <nav className="hidden auto-cols-fr grid-flow-col border-t border-border bg-background px-1 py-1.5 phone:grid">
        {[...NAV.filter((n) => n.to !== "/find" && n.to !== "/stats"), MORE].map(({ to, label, icon: Icon }) => (
          <Link
            key={to}
            to={to}
            onClick={toSection(to)}
            className="flex flex-col items-center gap-0.5 py-1 text-[11px]"
            activeProps={{ className: "text-brand-text" }}
            inactiveProps={{ className: "text-muted-foreground" }}
          >
            <span className="relative">
              <Icon className="size-[22px]" strokeWidth={1.6} />
              {to === "/conversations" && (
                <UnreadBadge className="absolute -top-1.5 left-3.5 min-w-4 px-1 text-[10px] leading-4" />
              )}
              {to === "/tasks" && (
                <ReviewCount className="absolute -top-1.5 left-3.5 min-w-4 px-1 text-[10px] leading-4" />
              )}
              {to === "/more" && <WaitingDot />}
            </span>
            {label}
          </Link>
        ))}
      </nav>
    </div>
  );
}

/**
 * docs/ui.md#布局: shows the sidebar as icons only, or in full. It stays where it is either way:
 * right of the macOS window's buttons, at the top of the sidebar elsewhere.
 */
function SidebarToggle({ collapsed, onClick }: { collapsed: boolean; onClick: () => void }) {
  const label = collapsed ? "展开侧边栏" : "收起侧边栏";
  return (
    <Tooltip content={label} side="bottom">
      <button
        type="button"
        aria-label={label}
        onClick={onClick}
        className={cn(
          "absolute z-10 grid place-items-center rounded-md text-muted-foreground hover:bg-press phone:hidden",
          titleBar > 0 ? "top-[calc(var(--title-bar)/2_-_14px)] left-[76px] size-7" : "top-4 left-3 h-10 w-[42px]",
        )}
      >
        <PanelLeft className="size-[18px]" strokeWidth={1.6} />
      </button>
    </Tooltip>
  );
}

/** Where a count sits with the sidebar collapsed: on the icon's corner, as on the phone's tabs. */
const ON_ICON = "absolute -top-1.5 left-2.5 min-w-4 px-1 text-[10px] leading-4";

/** An entry of the sidebar; collapsed, only its icon, with its name in a hint. */
function NavLink({
  to,
  label,
  icon: Icon,
  spin,
  count,
  collapsed,
  onClick,
}: {
  to: (typeof NAV)[number]["to"] | typeof SETTINGS.to | "/background";
  label: string;
  icon: LucideIcon;
  spin?: boolean;
  /** What the entry counts: new replies, letters to review, or what waits for the user. */
  count?: "unread" | "review" | "waiting";
  collapsed: boolean;
  onClick?: React.MouseEventHandler;
}) {
  const badge =
    count === "unread" ? (
      <UnreadBadge className={collapsed ? ON_ICON : "ml-auto"} />
    ) : count === "review" ? (
      <ReviewCount className={collapsed ? ON_ICON : "ml-auto"} />
    ) : (
      count === "waiting" && <WaitingCount onIcon={collapsed} />
    );
  return (
    <Tooltip content={collapsed ? label : undefined} side="right">
      <Link
        to={to}
        onClick={onClick}
        className="flex h-10 items-center gap-3 rounded-md px-3 whitespace-nowrap"
        activeProps={{ className: "bg-background font-medium text-brand-text" }}
        inactiveProps={{ className: "text-muted-foreground hover:bg-press" }}
      >
        <span className="relative shrink-0">
          <Icon className={cn("size-[18px]", spin && "animate-spin")} strokeWidth={1.6} />
          {collapsed && badge}
        </span>
        <span className={collapsed ? "sr-only" : "truncate"}>{label}</span>
        {!collapsed && badge}
      </Link>
    </Tooltip>
  );
}

/** How many mentors have new replies; a count that could not be read says so instead. */
function UnreadBadge({ className }: { className?: string }) {
  const unread = useQuery({ queryKey: ["unreadCount"], queryFn: () => call("unreadCount", {}) });
  if (!unread.isError && !unread.data) return null;
  const failed = unread.isError ? `新回信数量读取失败：${errorMessage(unread.error)}` : undefined;
  return (
    <Tooltip content={failed}>
      <span
        role={failed ? "img" : undefined}
        aria-label={failed}
        className={cn("min-w-5 rounded-full bg-unread px-1.5 text-center text-xs leading-5 text-white", className)}
      >
        {unread.isError ? "!" : unread.data > 99 ? "99+" : unread.data}
      </span>
    </Tooltip>
  );
}

/**
 * Letters in tasks waiting for review, in grey (docs/ui.md#交互): they wait for the user, but
 * nothing goes wrong while they do. Generation adds to them in the background.
 */
function ReviewCount({ className }: { className?: string }) {
  const count = useQuery({ queryKey: ["reviewCount"], queryFn: () => call("reviewCount", {}), refetchInterval: 3000 });
  if (!count.isError && !count.data) return null;
  const failed = count.isError ? `待审核数量读取失败：${errorMessage(count.error)}` : undefined;
  return (
    <Tooltip content={failed}>
      <span
        role={failed ? "img" : "status"}
        aria-label={failed ?? `${count.data} 封草稿待审核`}
        className={cn(
          "min-w-5 rounded-full bg-press px-1.5 text-center text-xs leading-5 text-muted-foreground",
          className,
        )}
      >
        {count.isError ? "!" : count.data > 99 ? "99+" : count.data}
      </span>
    </Tooltip>
  );
}

/** On a phone the background page is under "更多", and the tab lights up when something waits. */
function WaitingDot() {
  const { waiting } = useBackground();
  return (
    waiting > 0 && (
      <span
        role="status"
        aria-label={`${waiting} 项待处理`}
        className="absolute -top-0.5 -right-0.5 size-2 rounded-full bg-warning"
      />
    )
  );
}

/** The account row's label: the account's email once there is one. */
function useSettingsLabel() {
  const account = useAccount().data;
  return account && account.kind !== "loggedOut" ? account.email : SETTINGS.label;
}

function AccountRow({ collapsed, onClick }: { collapsed: boolean; onClick: React.MouseEventHandler }) {
  const label = useSettingsLabel();
  return <NavLink {...SETTINGS} label={label} collapsed={collapsed} onClick={onClick} />;
}

/** docs/ui.md#后台: turning while work runs, with how many things wait for the user. */
function BackgroundEntry({ collapsed, onClick }: { collapsed: boolean; onClick: React.MouseEventHandler }) {
  const work = useBackground();
  return (
    <NavLink
      to="/background"
      label="后台"
      icon={work.running > 0 ? LoaderCircle : Activity}
      spin={work.running > 0}
      collapsed={collapsed}
      count="waiting"
      onClick={onClick}
    />
  );
}

/** How many things wait for the user, in the background's yellow (docs/ui.md#交互). */
function WaitingCount({ onIcon }: { onIcon: boolean }) {
  const count = useBackground().waiting;
  if (count === 0) return null;
  const badge = (
    <span
      className={cn(
        "min-w-5 rounded-full bg-warning-soft px-1.5 text-center text-xs leading-5 text-warning",
        onIcon ? ON_ICON : "ml-auto",
      )}
    >
      {count}
    </span>
  );
  // On the icon, the entry's own hint shows instead.
  return onIcon ? badge : <Tooltip content={`${count} 项待处理`}>{badge}</Tooltip>;
}

export function MorePage() {
  const settings = useSettingsLabel();
  const work = useBackground();
  return (
    <div className="p-5">
      <h1 className="mb-5 text-2xl font-semibold">更多</h1>
      <Link to="/find" search={findSearch({})} className="mb-3 flex h-12 items-center gap-3 rounded-md bg-muted px-4">
        <Search className="size-5" />
        找导师
      </Link>
      <Link to="/stats" className="mb-3 flex h-12 items-center gap-3 rounded-md bg-muted px-4">
        <ChartNoAxesColumn className="size-5" />
        统计
      </Link>
      <Link to="/settings" className="mb-3 flex h-12 items-center gap-3 rounded-md bg-muted px-4">
        <Settings className="size-5 shrink-0" />
        <span className="truncate">{settings}</span>
      </Link>
      <Link to="/background" className="flex h-12 items-center gap-3 rounded-md bg-muted px-4">
        {work.running > 0 ? <LoaderCircle className="size-5 animate-spin" /> : <Activity className="size-5" />}
        后台
        <WaitingCount onIcon={false} />
      </Link>
    </div>
  );
}
