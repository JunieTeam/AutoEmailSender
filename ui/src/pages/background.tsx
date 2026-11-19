import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { BackButton } from "@/back";
import { Activity } from "lucide-react";
import { useState, type ReactNode } from "react";
import { cn } from "cn";
import { call, failureOf } from "@/api/call";
import { ModelAction } from "@/components/ai-writing";
import { FailureDetails, FailureNote } from "@/components/failure";
import { LibraryUpdatesDialog } from "@/components/library";
import { refreshMentors, refreshReplyOutcomes } from "@/components/mentor";
import { Notice } from "@/components/notice";
import { scheduledAt } from "@/components/schedule";
import { refreshTasks } from "@/components/task-cache";
import { Button } from "@/components/ui/button";
import { useReceive } from "@/pages/conversations";
import { crawlName, findSearch, refreshCrawls, ResumeCrawl } from "@/pages/find";
import { useResume } from "@/pages/mailboxes";

/** Today in the user's time zone, which the core cannot know. */
function localToday() {
  const now = new Date();
  return {
    start: new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime(),
    end: new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime(),
  };
}

export function useBackground() {
  const tasks = useQuery({
    queryKey: ["taskWork"],
    queryFn: () => call("taskWork", {}),
    refetchInterval: 3000,
  });
  const matching = useQuery({
    queryKey: ["matchingProgress"],
    queryFn: () => call("matchingProgress", {}),
    refetchInterval: 3000,
  });
  const crawls = useQuery({
    queryKey: ["crawlWork"],
    queryFn: () => call("crawlWork", localToday()),
    refetchInterval: 3000,
  });
  const done = useQuery({
    queryKey: ["todayWork"],
    queryFn: () => call("todayWork", localToday()),
    refetchInterval: 3000,
  });
  const scheduled = useQuery({
    queryKey: ["scheduledWork"],
    queryFn: () => call("scheduledWork", {}),
    refetchInterval: 3000,
  });
  const replies = useQuery({
    queryKey: ["replyProcessing"],
    queryFn: () => call("replyProcessing", {}),
    refetchInterval: 3000,
  });
  const mailboxes = useQuery({
    queryKey: ["listMailboxes"],
    queryFn: () => call("listMailboxes", {}),
    refetchInterval: 3000,
  });
  const library = useQuery({
    queryKey: ["libraryUpdateCount"],
    queryFn: () => call("libraryUpdateCount", {}),
    // Only a synchronization, at most hourly, adds to it; changes the user makes refresh it directly.
    refetchInterval: 60_000,
  });
  const cloud = useQuery({
    queryKey: ["getCloudBackup"],
    queryFn: () => call("getCloudBackup", {}),
    // Backups run at most hourly; the backup page refreshes it after its own.
    refetchInterval: 60_000,
  });
  const libraryFailure = useQuery({
    queryKey: ["libraryFailure"],
    queryFn: () => call("libraryFailure", {}),
    // Synchronization runs at most hourly.
    refetchInterval: 60_000,
  });
  const storage = useQuery({
    queryKey: ["storageFailure"],
    queryFn: () => call("storageFailure", {}),
    refetchInterval: 3000,
  });
  const queries = [
    tasks,
    matching,
    crawls,
    done,
    scheduled,
    replies,
    mailboxes,
    library,
    cloud,
    libraryFailure,
    storage,
  ];
  const today = new Date(crawls.dataUpdatedAt).toDateString();
  const work = {
    storage: storage.data ?? null,
    // Failed and unsendable letters, whichever task they are in, wait together in the sending plan.
    emails: scheduled.data?.pending ?? 0,
    mailboxes: mailboxes.data?.flatMap(({ id, address, pause }) => (pause ? [{ id, address, pause }] : [])) ?? [],
    // Only waiting for its time, or for the secret to be fixed here.
    cloudPaused:
      mailboxes.data?.flatMap(({ id, address, cloudPause, cloudEmails }) =>
        cloudPause ? [{ id, address, pause: cloudPause, count: cloudEmails }] : [],
      ) ?? [],
    // A round that could not reach a mailbox still counts as received; the reason stays on the mailbox.
    unreceived:
      mailboxes.data?.flatMap(({ id, address, receiveError }) =>
        receiveError ? [{ id, address, receiveError }] : [],
      ) ?? [],
    paused: tasks.data?.filter((t) => t.state === "paused") ?? [],
    crawlsPending: crawls.data?.filter((c) => c.state === "paused" || c.state === "failed" || c.failed > 0) ?? [],
    crawling: crawls.data?.filter((c) => c.state === "running") ?? [],
    generating: tasks.data?.filter((t) => t.generating > 0 && t.state !== "paused") ?? [],
    matching: matching.data?.remaining ?? 0,
    matchingPause: matching.data?.pause ?? null,
    receiving: (mailboxes.data?.length ?? 0) > 0,
    repliesPaused: replies.data?.pause ?? null,
    libraryUpdates: library.data ?? 0,
    cloudFailure: lasting(cloud.data?.enabled ? cloud.data.failure : null, cloud.dataUpdatedAt),
    libraryFailure: lasting(libraryFailure.data, libraryFailure.dataUpdatedAt),
    marking: replies.data?.pause ? 0 : (replies.data?.remaining ?? 0),
    crawled:
      crawls.data?.filter((c) => c.state === "done" && new Date(c.finishedAt ?? 0).toDateString() === today) ?? [],
    drafted: done.data?.drafts ?? [],
    matched: done.data?.matched ?? 0,
    sent: done.data?.sent ?? 0,
    next: scheduled.data?.nextAt ?? null,
    nextToday: scheduled.data?.today ?? 0,
    error:
      storage.error ??
      tasks.error ??
      matching.error ??
      scheduled.error ??
      crawls.error ??
      mailboxes.error ??
      replies.error ??
      library.error ??
      libraryFailure.error ??
      cloud.error ??
      done.error,
  };
  return {
    ...work,
    pending: queries.some((q) => q.isPending),
    finished: work.crawled.length + work.drafted.length + (work.matched > 0 ? 1 : 0) + (work.sent > 0 ? 1 : 0),
    running:
      work.crawling.length +
      work.generating.length +
      (work.matching > 0 && !work.matchingPause ? 1 : 0) +
      (work.marking > 0 ? 1 : 0),
    waiting:
      (work.storage ? 1 : 0) +
      (work.emails > 0 ? 1 : 0) +
      work.mailboxes.length +
      work.cloudPaused.length +
      work.unreceived.length +
      work.paused.length +
      work.crawlsPending.length +
      (work.matching > 0 && work.matchingPause ? 1 : 0) +
      (work.repliesPaused ? 1 : 0) +
      (work.libraryUpdates > 0 ? 1 : 0) +
      (work.cloudFailure ? 1 : 0) +
      (work.libraryFailure ? 1 : 0),
  };
}

/** Work retried hourly in the background; only a failure lasting days needs the user. */
function lasting(failure: { since: number; message: string } | null | undefined, now: number) {
  const days = failure ? Math.floor((now - failure.since) / 86_400_000) : 0;
  return failure && days >= 3 ? { days, message: failure.message } : null;
}

/** docs/ui.md#后台: what runs in the background and what of it waits for the user, each leading to where it is. */
export function BackgroundPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const work = useBackground();
  const crawl = useMutation({
    mutationFn: ({ op, id }: { op: "pauseCrawl" | "resumeCrawl"; id: number }) => call(op, { id }),
    onSuccess: () => refreshCrawls(queryClient),
  });
  const matching = useMutation({
    mutationFn: (op: "pauseMatching" | "resumeMatching") => call(op, {}),
    onSettled: () => refreshMentors(queryClient),
  });
  const task = useMutation({
    mutationFn: ({ op, id }: { op: "pauseTask" | "resumeTask"; id: number }) => call(op, { id }),
    onSuccess: () => refreshTasks(queryClient),
  });
  const resumeReplies = useMutation({
    mutationFn: () => call("resumeReplyProcessing", {}),
    onSuccess: () => refreshReplyOutcomes(queryClient),
  });
  const resume = useResume();
  const { receive, receiving, receiveError, received } = useReceive();
  const [libraryUpdates, setLibraryUpdates] = useState(false);
  const error =
    work.error ?? crawl.error ?? matching.error ?? task.error ?? resume.error ?? resumeReplies.error ?? receiveError;
  const idle = work.waiting === 0 && work.running === 0 && work.next === null && !work.receiving && work.finished === 0;
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-[720px] flex-col gap-7 px-7 pt-6 pb-8 phone:px-4 phone:pt-3.5">
        <div className="flex items-center gap-1">
          <BackButton
            up={() => void navigate({ to: "/more", replace: true })}
            className="-ml-2 hidden phone:inline-flex"
          />
          <h1 className="text-2xl font-semibold phone:text-[22px]">后台</h1>
        </div>
        {work.waiting > 0 && (
          <Group title="待处理" className="text-warning">
            {work.storage && (
              <Link to="/settings/about" className={ITEM}>
                <Entry title="数据暂时无法保存" detail={work.storage} />
              </Link>
            )}
            {work.emails > 0 && (
              <Link to="/tasks" search={{ tab: "plan" }} className={ITEM}>
                <Entry title={`${work.emails} 封邮件待处理`} detail="去发送计划里处理" />
              </Link>
            )}
            {work.mailboxes.map((m) => (
              <div key={m.id}>
                <div className="flex items-center gap-2">
                  <Link to="/settings/mailboxes" className={ITEM}>
                    <Entry
                      title={m.address}
                      detail={`发信暂停到 ${scheduledAt(m.pause.until)}：${m.pause.reason.message}`}
                    />
                  </Link>
                  <Button variant="quiet" size="sm" disabled={resume.isPending} onClick={() => resume.mutate(m.id)}>
                    恢复
                  </Button>
                </div>
                <FailureDetails failure={m.pause.reason} />
              </div>
            ))}
            {work.cloudPaused.map((m) => (
              <div key={m.id}>
                <Link to="/settings/mailboxes" className={ITEM}>
                  <Entry
                    title={m.address}
                    detail={`云端发信暂停到 ${scheduledAt(m.pause.until)}，${m.count} 封在等：${m.pause.reason.message}`}
                  />
                </Link>
                <FailureDetails failure={m.pause.reason} />
              </div>
            ))}
            {work.unreceived.map((m) => (
              <div key={m.id}>
                <Link to="/settings/mailboxes" className={ITEM}>
                  <Entry title={m.address} detail={`收信失败：${m.receiveError.message}`} />
                </Link>
                <FailureDetails failure={m.receiveError} />
              </div>
            ))}
            {work.paused.map((t) => (
              <div key={t.id}>
                <div className="flex items-center gap-2">
                  <Link to="/tasks/$id" params={{ id: t.id }} className={ITEM}>
                    <Entry title={t.name} detail={t.pauseReason?.message ?? "已暂停"} />
                  </Link>
                  {/* Going on writes the letters still to write; with none left, it calls no model. */}
                  {t.generating > 0 ? (
                    <ModelAction
                      feature="writing"
                      variant="quiet"
                      size="sm"
                      modelIds={[t.modelConfigId]}
                      checked
                      disabled={task.isPending}
                      onWrite={() => task.mutate({ op: "resumeTask", id: t.id })}
                    >
                      继续
                    </ModelAction>
                  ) : (
                    <Button
                      variant="quiet"
                      size="sm"
                      disabled={task.isPending}
                      onClick={() => task.mutate({ op: "resumeTask", id: t.id })}
                    >
                      继续
                    </Button>
                  )}
                </div>
                {t.pauseReason && <FailureDetails failure={t.pauseReason} />}
              </div>
            ))}
            {work.crawlsPending.map((c) => (
              <div key={c.id}>
                <div className="flex items-center gap-2">
                  <Link to="/find" search={findSearch({ tab: "crawl", detail: c.id })} className={ITEM}>
                    <Entry
                      title={crawlName(c)}
                      detail={c.error?.message ?? (c.state === "paused" ? "已暂停" : `${c.failed} 个页面未能打开`)}
                    />
                  </Link>
                  {c.state === "paused" && (
                    <ResumeCrawl
                      crawl={c}
                      size="sm"
                      disabled={crawl.isPending}
                      onResume={() => crawl.mutate({ op: "resumeCrawl", id: c.id })}
                    />
                  )}
                </div>
                {c.error && <FailureDetails failure={c.error} />}
              </div>
            ))}
            {work.matching > 0 && work.matchingPause && (
              <div>
                <div className="flex items-center gap-2">
                  <Link to="/mentors" className={ITEM}>
                    <Entry
                      title="匹配度分析已暂停"
                      detail={work.matchingPause.reason?.message ?? `还剩 ${work.matching} 位`}
                    />
                  </Link>
                  <ModelAction
                    feature="match"
                    variant="quiet"
                    size="sm"
                    disabled={matching.isPending}
                    onWrite={() => matching.mutate("resumeMatching")}
                  >
                    继续
                  </ModelAction>
                </div>
                {work.matchingPause.reason && <FailureDetails failure={work.matchingPause.reason} />}
              </div>
            )}
            {work.repliesPaused && (
              <div>
                <div className="flex items-center gap-2">
                  <Link to="/conversations" className={ITEM}>
                    <Entry title="回信处理已暂停" detail={work.repliesPaused.reason.message} />
                  </Link>
                  <ModelAction
                    feature="reply"
                    variant="quiet"
                    size="sm"
                    disabled={resumeReplies.isPending}
                    onWrite={() => resumeReplies.mutate()}
                  >
                    继续
                  </ModelAction>
                </div>
                <FailureDetails failure={work.repliesPaused.reason} />
              </div>
            )}
            {work.libraryUpdates > 0 && (
              <button className={cn(ITEM, "w-full text-left")} onClick={() => setLibraryUpdates(true)}>
                <Entry title={`导师库有 ${work.libraryUpdates} 位导师的资料有更新`} detail="打开去选" />
              </button>
            )}
            {work.cloudFailure && (
              <Link to="/settings/backup" className={ITEM}>
                <Entry title={`云端备份已连续 ${work.cloudFailure.days} 天未成功`} detail={work.cloudFailure.message} />
              </Link>
            )}
            {work.libraryFailure && (
              <Link to="/find" search={findSearch({ tab: "library" })} className={ITEM}>
                <Entry
                  title={`导师库已连续 ${work.libraryFailure.days} 天未能同步`}
                  detail={work.libraryFailure.message}
                />
              </Link>
            )}
          </Group>
        )}
        {work.running > 0 && (
          <Group title="进行中">
            {work.crawling.map((c) => (
              <div key={c.id} className="flex items-center gap-2">
                <Link to="/find" search={findSearch({ tab: "crawl", detail: c.id })} className={ITEM}>
                  {c.stage === "enrich" ? (
                    <Entry title="信息补全" detail={`还剩 ${c.enrichRemaining} 位`} />
                  ) : (
                    <Entry
                      title={crawlName(c)}
                      detail={`已读 ${c.pages} 页，找到 ${c.candidates} 位候选${c.enrichRemaining > 0 ? `，补全还剩 ${c.enrichRemaining} 位` : ""}`}
                    />
                  )}
                </Link>
                <Button
                  variant="quiet"
                  size="sm"
                  disabled={crawl.isPending}
                  onClick={() => crawl.mutate({ op: "pauseCrawl", id: c.id })}
                >
                  暂停
                </Button>
              </div>
            ))}
            {work.generating.map((t) => (
              <div key={t.id} className="flex items-center gap-2">
                <Link to="/tasks/$id" params={{ id: t.id }} className={ITEM}>
                  <Entry title={t.name} detail={`生成草稿，还剩 ${t.generating} 封`} />
                </Link>
                <Button
                  variant="quiet"
                  size="sm"
                  disabled={task.isPending}
                  onClick={() => task.mutate({ op: "pauseTask", id: t.id })}
                >
                  暂停
                </Button>
              </div>
            ))}
            {work.matching > 0 && !work.matchingPause && (
              <div className="flex items-center gap-2">
                <Link to="/mentors" className={ITEM}>
                  <Entry title="匹配度分析" detail={`还剩 ${work.matching} 位`} />
                </Link>
                <Button
                  variant="quiet"
                  size="sm"
                  disabled={matching.isPending}
                  onClick={() => matching.mutate("pauseMatching")}
                >
                  暂停
                </Button>
              </div>
            )}
            {work.marking > 0 && (
              <Link to="/conversations" className={ITEM}>
                <Entry title="处理回信" detail={`还剩 ${work.marking} 项`} />
              </Link>
            )}
          </Group>
        )}
        {(work.next !== null || work.receiving) && (
          <Group title="按计划进行">
            {work.next !== null && (
              <Link to="/tasks" search={{ tab: "plan", column: "upcoming" }} className={ITEM}>
                <Entry title={`下一封 ${scheduledAt(work.next)}`} detail={`今天还剩 ${work.nextToday} 封`} />
              </Link>
            )}
            {work.receiving && (
              <div className="flex items-center gap-2">
                <Link to="/conversations" className={ITEM}>
                  <Entry title="收信" detail={received ?? "尚未收信"} />
                </Link>
                <Button variant="quiet" size="sm" disabled={receiving} onClick={receive}>
                  立即收信
                </Button>
              </div>
            )}
          </Group>
        )}
        {work.finished > 0 && (
          <Group title="今天完成">
            {work.crawled.map((c) => (
              <Link key={c.id} to="/find" search={findSearch({ crawl: c.id })} className={ITEM}>
                <Entry
                  title={crawlName(c)}
                  detail={
                    c.kind === "mentors" ? `补全 ${c.candidates} 位，去审核` : `找到 ${c.candidates} 位候选，去审核`
                  }
                />
              </Link>
            ))}
            {work.drafted.map((t) => (
              <Link key={t.id} to="/tasks/$id" params={{ id: t.id }} className={ITEM}>
                <Entry title={t.name} detail={`生成 ${t.generated} 封草稿，去任务`} />
              </Link>
            ))}
            {work.matched > 0 && (
              <Link to="/mentors" className={ITEM}>
                <Entry title="匹配度分析" detail={`分析了 ${work.matched} 位，去导师列表`} />
              </Link>
            )}
            {work.sent > 0 && (
              <Link to="/tasks" search={{ tab: "plan", column: "history" }} className={ITEM}>
                <Entry title="发信" detail={`发出 ${work.sent} 封，去发送历史`} />
              </Link>
            )}
          </Group>
        )}
        {idle && !work.pending && (
          <div className="flex flex-1 flex-col justify-center">
            <Notice
              icon={Activity}
              title="当前无后台工作"
              detail="匹配分析、智能整理、生成草稿、定时发送和收信开始后，会在这里显示进度和结果"
            />
          </div>
        )}
        {error && <FailureNote className="text-sm text-destructive" failure={failureOf(error)} />}
      </div>
      {libraryUpdates && <LibraryUpdatesDialog onClose={() => setLibraryUpdates(false)} />}
    </div>
  );
}

function Group({ title, className, children }: { title: string; className?: string; children: ReactNode }) {
  return (
    <section>
      <h2 className={cn("mb-1.5 text-sm font-medium text-muted-foreground", className)}>{title}</h2>
      {children}
    </section>
  );
}

const ITEM = "-mx-3 block min-w-0 grow rounded-md px-3 py-2 hover:bg-muted";

function Entry({ title, detail }: { title: string; detail: string }) {
  return (
    <>
      <span className="block truncate">{title}</span>
      <span className="block text-sm text-muted-foreground">{detail}</span>
    </>
  );
}
