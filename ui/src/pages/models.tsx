import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { call, failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type { FeatureModel, ModelConfig, ModelFeature, ModelProvider } from "@/api/types";
import { Confirm } from "@/components/batch-bar";
import { ModelDialog, sourceName, useModelProviders } from "@/components/model-dialog";
import { DefaultMark } from "@/components/default-mark";
import { Button } from "@/components/ui/button";
import { Select, type Options } from "@/components/ui/select";
import { Row, Section } from "./settings";

/** Settings › AI › 模型 (docs/ui.md#交互). */
export function ModelsSection() {
  const configs = useQuery({ queryKey: ["listModelConfigs"], queryFn: () => call("listModelConfigs", {}) });
  const providers = useModelProviders();
  const [adding, setAdding] = useState(false);
  const error = configs.error ?? providers.error;
  return (
    <Section title="模型">
      <p className="text-sm text-muted-foreground">
        分析匹配度、写邮件草稿用的 AI 模型，可以添加多个。API Key 只存在这台电脑上，只发给对应的服务商。
      </p>
      {error && <FailureNote className="text-sm text-destructive" lead="模型未能加载：" failure={failureOf(error)} />}
      {configs.data && providers.data && (
        <>
          {configs.data.length > 0 && (
            <ul className="flex flex-col divide-y divide-border">
              {configs.data.map((c) => (
                <ModelRow key={c.id} config={c} providers={providers.data} />
              ))}
            </ul>
          )}
          <div>
            <Button onClick={() => setAdding(true)}>添加模型</Button>
          </div>
          {adding && <ModelDialog config={null} providers={providers.data} onClose={() => setAdding(false)} />}
          {configs.data.length > 1 && <FeatureModels configs={configs.data} providers={providers.data} />}
        </>
      )}
    </Section>
  );
}

function ModelRow({ config: c, providers }: { config: ModelConfig; providers: ModelProvider[] }) {
  const [editing, setEditing] = useState(false);
  const queryClient = useQueryClient();
  const test = useMutation({
    mutationFn: () => call("testModel", { ...c, id: c.id, name: "", baseUrl: c.baseUrl ?? "", apiKey: "" }),
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["listModelConfigs"] });
  const remove = useMutation({ mutationFn: () => call("deleteModelConfig", { id: c.id }), onSuccess: refresh });
  const makeDefault = useMutation({ mutationFn: () => call("setDefaultModel", { id: c.id }), onSuccess: refresh });
  const source = sourceName(c, providers);
  return (
    <li className="flex flex-col gap-1.5 py-3">
      <div className="flex flex-wrap items-center gap-2 phone:flex-col phone:items-start">
        <div className="max-w-full min-w-40 grow">
          <p className="flex items-center gap-2 font-medium">
            <span className="truncate">{c.name ?? c.model}</span>
            {c.isDefault && <DefaultMark />}
          </p>
          <p className="truncate text-sm text-muted-foreground">{c.name ? `${source}，${c.model}` : source}</p>
          {c.needsApiKey && <p className="text-sm text-warning">恢复后需要重新填写 API Key，填好之前无法使用</p>}
        </div>
        <div className="flex flex-wrap gap-1">
          <Button variant="quiet" disabled={test.isPending} onClick={() => test.mutate()}>
            测试
          </Button>
          {!c.isDefault && (
            <Button variant="ghost" disabled={makeDefault.isPending} onClick={() => makeDefault.mutate()}>
              设为默认
            </Button>
          )}
          <Button variant="ghost" onClick={() => setEditing(true)}>
            编辑
          </Button>
          <Confirm
            trigger={
              <Button variant="danger" disabled={remove.isPending}>
                删除
              </Button>
            }
            message="删除这个模型？用过它的记录里还保留模型名。"
            action="删除"
            side="bottom"
            onConfirm={() => remove.mutate()}
          />
        </div>
      </div>
      <div className="text-sm empty:hidden" aria-live="polite">
        {test.isPending && <span className="text-muted-foreground">正在测试…</span>}
        {test.isSuccess && <span className="text-muted-foreground">模型回复正常</span>}
        {test.isError && <FailureNote className="text-destructive" failure={failureOf(test.error)} />}
        {remove.isError && <FailureNote className="text-destructive" failure={failureOf(remove.error)} />}
        {makeDefault.isError && <FailureNote className="text-destructive" failure={failureOf(makeDefault.error)} />}
      </div>
      {editing && (
        <ModelDialog
          config={c}
          providers={providers}
          // A test of the old settings says nothing about the new ones.
          onSaved={() => test.reset()}
          onClose={() => setEditing(false)}
        />
      )}
    </li>
  );
}

// docs/ui.md#交互: the features whose model can be chosen apart from the default.
const FEATURES: readonly (readonly [ModelFeature, string, string?])[] = [
  ["crawl", "智能整理", "读网页、整理导师资料，也用于信息补全"],
  ["match", "匹配分析"],
  ["writing", "AI 写信", "起草和改写，新建任务时用它"],
  ["check", "AI 核对"],
  ["reply", "回信处理", "标记回信结果、提取待办"],
];

/** Which configuration each feature calls, saved as soon as it is chosen. */
function FeatureModels({ configs, providers }: { configs: ModelConfig[]; providers: ModelProvider[] }) {
  const queryClient = useQueryClient();
  const save = useMutation({
    mutationFn: (input: FeatureModel) => call("setFeatureModel", input),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["listModelConfigs"] }),
  });
  const fallback = configs.find((c) => c.isDefault);
  const options: Options<number | null> = [
    [null, `跟随默认（${fallback?.name ?? fallback?.model ?? ""}）`],
    ...configs.map((c): [number, ReactNode] => [
      c.id,
      <>
        {c.name ?? c.model}
        <span className="ml-2 text-muted-foreground">{sourceName(c, providers)}</span>
      </>,
    ]),
  ];
  return (
    <Row label="各功能用的模型">
      <p className="text-sm text-muted-foreground">没有单独选的功能用默认的模型</p>
      <div className="mt-1.5 flex flex-col gap-3">
        {FEATURES.map(([feature, label, detail]) => (
          <div
            key={feature}
            className="flex items-center justify-between gap-4 phone:flex-col phone:items-stretch phone:gap-1.5"
          >
            <div className="min-w-0">
              <p>{label}</p>
              {detail && <p className="text-sm text-muted-foreground">{detail}</p>}
            </div>
            <Select
              aria-label={label}
              className="w-64 shrink-0 phone:w-full"
              value={configs.find((c) => c.features.includes(feature))?.id ?? null}
              options={options}
              disabled={save.isPending}
              onChange={(id) => save.mutate({ feature, id })}
            />
          </div>
        ))}
      </div>
      {save.error && (
        <FailureNote className="text-sm text-destructive" lead="未能保存：" failure={failureOf(save.error)} />
      )}
    </Row>
  );
}
