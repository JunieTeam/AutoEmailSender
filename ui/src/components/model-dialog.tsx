import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useRef, useState } from "react";
import { call, failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type {
  Currency,
  ListedModel,
  ModelConfig,
  ModelPrices,
  ModelProvider,
  ModelSource,
  ModelTest,
  NewModelConfig,
  PeakHours,
} from "@/api/types";
import { formClose, Field, TestResult } from "@/components/form";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogClose, DialogContent, DialogFooter, DialogHeader } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Autocomplete } from "@/components/ui/autocomplete";
import { NumberField } from "@/components/ui/number-field";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Select } from "@/components/ui/select";
import { UNITS } from "@/format";

/** The provider table from the core; it never changes while the app runs. */
export function useModelProviders() {
  return useQuery({
    queryKey: ["listModelProviders"],
    queryFn: () => call("listModelProviders", {}),
    staleTime: Infinity,
  });
}

const CUSTOM = "custom";

/**
 * Where a configuration calls, as the user knows it: its provider, with the region where it has
 * several, or else its address.
 */
export function sourceName(c: Pick<ModelConfig, "provider" | "region" | "baseUrl">, providers: ModelProvider[]) {
  const provider = providers.find((p) => p.id === c.provider);
  if (!provider) return c.baseUrl ?? c.provider;
  const region = provider.endpoints.find((e) => e.region !== null && e.region.id === c.region)?.region;
  return region ? `${provider.name}${region.name}地域` : provider.name;
}

const NO_PRICES: ModelPrices = { input: null, cache: null, output: null };

/** The form always names its currency, which the core would otherwise take as the provider's, and prices. */
type Form = Omit<Required<NewModelConfig>, "currency"> & { currency: Currency };

const EMPTY: Form = {
  name: "",
  provider: "",
  region: null,
  baseUrl: "",
  model: "",
  apiKey: "",
  currency: "cny",
  prices: NO_PRICES,
  offPeak: NO_PRICES,
};

function formOf(c: ModelConfig): Form {
  return {
    name: c.name ?? "",
    provider: c.provider,
    region: c.region,
    baseUrl: c.baseUrl ?? "",
    model: c.model,
    apiKey: "",
    currency: c.currency,
    prices: c.prices,
    offPeak: c.offPeak,
  };
}

type Pricing = Pick<Form, "currency" | "prices" | "offPeak">;

/**
 * The official prices of the model the form names, where they are known: those in the provider
 * table, which are for the provider's first region, or those its model list gave.
 */
function officialPricing(
  form: Form,
  provider: ModelProvider | undefined,
  listed: ListedModel[] | undefined,
): Pricing | undefined {
  if (!provider || form.region !== (provider.endpoints[0]?.region?.id ?? null)) return undefined;
  const model = form.model.trim();
  const known = provider.models.find((m) => m.name === model);
  if (known) return { currency: provider.currency, prices: known.prices, offPeak: known.offPeak ?? NO_PRICES };
  const prices = listed?.find((m) => m.name === model)?.prices;
  return prices ? { currency: provider.currency, prices, offPeak: NO_PRICES } : undefined;
}

function samePrices(a: ModelPrices, b: ModelPrices) {
  return a.input === b.input && a.cache === b.cache && a.output === b.output;
}

function samePricing(a: Pricing, b: Pricing) {
  return a.currency === b.currency && samePrices(a.prices, b.prices) && samePrices(a.offPeak, b.offPeak);
}

/** Adding or editing a model configuration (docs/ui.md#交互). Mounted while open. */
export function ModelDialog({
  config,
  providers,
  onSaved,
  onClose,
}: {
  config: ModelConfig | null;
  providers: ModelProvider[];
  /** After saving, before closing. */
  onSaved?: () => void;
  onClose: () => void;
}) {
  const [initial] = useState(() => (config ? formOf(config) : EMPTY));
  const [form, setForm] = useState(initial);
  const queryClient = useQueryClient();
  const save = useMutation({
    mutationFn: (f: Form) => (config ? call("updateModelConfig", { ...f, id: config.id }) : call("addModelConfig", f)),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["listModelConfigs"] });
      onSaved?.();
      onClose();
    },
  });
  const test = useMutation({ mutationFn: (t: ModelTest) => call("testModel", t) });
  const models = useMutation({ mutationFn: (s: ModelSource) => call("listProviderModels", s) });
  const providerRef = useRef<HTMLButtonElement>(null);

  const provider = providers.find((p) => p.id === form.provider);
  const endpoint = provider?.endpoints.find((e) => (e.region?.id ?? null) === form.region);
  const regions = provider?.endpoints.flatMap((e) => (e.region ? [[e.region.id, e.region.name] as const] : [])) ?? [];
  // The core only uses the saved key where it was saved for, and asks again for one a restore
  // left out only there.
  const sameSource =
    config !== null &&
    form.provider === initial.provider &&
    form.region === initial.region &&
    (form.provider !== CUSTOM || form.baseUrl === initial.baseUrl);
  const keySaved = sameSource && !config.needsApiKey;
  const keyLost = sameSource && config.needsApiKey;
  const reachable =
    (provider !== undefined || (form.provider === CUSTOM && form.baseUrl.trim() !== "")) &&
    (keySaved || (form.provider === CUSTOM && !keyLost) || form.apiKey.trim() !== "");
  const complete = reachable && form.model.trim() !== "";
  const dirty = JSON.stringify(form) !== JSON.stringify(initial);
  const names = [...new Set([...(provider?.models ?? []), ...(models.data ?? [])].map((m) => m.name))];

  /**
   * Changes the form. Prices are in the provider's currency, and only one with peak hours has
   * off-peak prices. They follow a known model: its official prices replace the others, and go
   * again, if left as they were, for a model, provider or region whose prices are not known. An
   * earlier test no longer applies, nor a list from another provider or key.
   */
  function edit(change: Partial<Form>) {
    let next = { ...form, ...change };
    const nextProvider = providers.find((p) => p.id === next.provider);
    if (next.provider !== form.provider) {
      next = {
        ...next,
        region: nextProvider?.endpoints[0]?.region?.id ?? null,
        currency: nextProvider?.currency ?? "cny",
        offPeak: nextProvider?.peakHours ? next.offPeak : NO_PRICES,
      };
    }
    const moved = next.provider !== form.provider || next.region !== form.region;
    if (moved || next.model !== form.model) {
      const before = officialPricing(form, provider, models.data);
      const after = officialPricing(next, nextProvider, moved ? undefined : models.data);
      if (after) next = { ...next, ...after };
      else if (before && samePricing(before, form)) next = { ...next, prices: NO_PRICES, offPeak: NO_PRICES };
    }
    setForm(next);
    test.reset();
    if (moved || next.baseUrl !== form.baseUrl || next.apiKey !== form.apiKey) models.reset();
  }

  const pending = save.isPending || test.isPending;
  const address = endpoint && (
    <p className="mt-1.5 text-sm break-all text-muted-foreground">接口地址：{endpoint.baseUrl}</p>
  );
  return (
    <Dialog open onOpenChange={formClose(dirty, save.isPending, onClose)}>
      <DialogContent initialFocus={providerRef}>
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate(form);
          }}
        >
          <DialogHeader>{config ? "编辑模型" : "添加模型"}</DialogHeader>
          <DialogBody>
            <fieldset disabled={save.isPending} className="flex min-w-0 flex-col gap-3.5">
              <Field label="服务商" id="model-provider">
                <Select
                  ref={providerRef}
                  id="model-provider"
                  value={form.provider}
                  placeholder="选择服务商"
                  options={[
                    ...providers.map((p) => [p.id, p.name] as const),
                    [CUSTOM, "其他（OpenAI 兼容接口）"] as const,
                  ]}
                  onChange={(id) => edit({ provider: id })}
                />
                {regions.length === 0 && address}
              </Field>
              {regions.length > 0 && (
                <Field label="地域" id="model-region">
                  <Select
                    id="model-region"
                    value={form.region}
                    options={regions}
                    onChange={(region) => edit({ region })}
                  />
                  {address}
                </Field>
              )}
              {form.provider === CUSTOM && (
                <Field label="接口地址" id="model-base-url">
                  <Input
                    id="model-base-url"
                    type="url"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="https://example.com/v1"
                    value={form.baseUrl}
                    onChange={(e) => edit({ baseUrl: e.target.value })}
                  />
                </Field>
              )}
              <Field label="API Key" id="model-api-key">
                <Input
                  id="model-api-key"
                  type="password"
                  autoComplete="off"
                  placeholder={
                    keySaved
                      ? "留空则不修改"
                      : keyLost
                        ? "从备份恢复后需要重新填写"
                        : form.provider === CUSTOM
                          ? "本机的模型可以不填"
                          : undefined
                  }
                  value={form.apiKey}
                  onChange={(e) => edit({ apiKey: e.target.value })}
                />
                {endpoint && (
                  <button
                    type="button"
                    className="mt-1.5 text-sm text-brand-text hover:underline"
                    onClick={() => void openUrl(endpoint.keyUrl)}
                  >
                    获取 API Key
                  </button>
                )}
              </Field>
              <Field label="模型名" id="model-model">
                <div className="flex gap-2">
                  <Autocomplete
                    id="model-model"
                    autoComplete="off"
                    spellCheck={false}
                    items={names}
                    value={form.model}
                    onValueChange={(model) => edit({ model })}
                  />
                  <Button
                    type="button"
                    variant="quiet"
                    disabled={!reachable || models.isPending}
                    onClick={() => models.mutate({ ...form, id: config?.id })}
                  >
                    获取列表
                  </Button>
                </div>
                <div className="mt-1.5 text-sm empty:hidden" aria-live="polite">
                  {models.isPending && <span className="text-muted-foreground">正在获取模型列表…</span>}
                  {models.isSuccess && (
                    <span className="text-muted-foreground">
                      {models.data.length > 0
                        ? `获取到 ${models.data.length} 个模型，点输入框选择或输入名称筛选`
                        : "服务商未列出模型，请手动填写"}
                    </span>
                  )}
                  {models.isError && <FailureNote className="text-destructive" failure={failureOf(models.error)} />}
                </div>
              </Field>
              <div role="group" aria-labelledby="model-prices">
                <div className="mb-1.5 flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
                  <span id="model-prices" className="text-sm text-muted-foreground">
                    单价（{UNITS[form.currency]}／百万 Token）
                  </span>
                  <RadioGroup
                    aria-label="单价的币种"
                    value={form.currency}
                    onValueChange={(currency) => edit({ currency })}
                    className="flex gap-4 text-sm"
                  >
                    {(["cny", "usd"] as const).map((c) => (
                      <label key={c} className="flex items-center gap-2">
                        <RadioGroupItem value={c} />
                        {UNITS[c]}
                      </label>
                    ))}
                  </RadioGroup>
                </div>
                {provider?.peakHours ? (
                  <div className="flex flex-col gap-3">
                    <div>
                      <p className="mb-1.5 text-sm">高峰时段：{hours(provider.peakHours)}</p>
                      <PriceFields
                        id="model-peak"
                        label="高峰时段"
                        prices={form.prices}
                        onChange={(prices) => edit({ prices })}
                      />
                    </div>
                    <div>
                      <p className="mb-1.5 text-sm">空闲时段：其余时间，不填时按高峰时段估算</p>
                      <PriceFields
                        id="model-off-peak"
                        label="空闲时段"
                        prices={form.offPeak}
                        onChange={(offPeak) => edit({ offPeak })}
                      />
                    </div>
                  </div>
                ) : (
                  <PriceFields id="model-price" prices={form.prices} onChange={(prices) => edit({ prices })} />
                )}
              </div>
              <Field label="名称" id="model-name">
                <Input
                  id="model-name"
                  autoComplete="off"
                  placeholder="可以不填，不填时显示模型名"
                  value={form.name}
                  onChange={(e) => edit({ name: e.target.value })}
                />
              </Field>
            </fieldset>
          </DialogBody>
          {test.status !== "idle" && (
            <div className="px-6 pt-3 text-sm phone:px-4" aria-live="polite">
              <TestResult label="测试" mutation={test} />
            </div>
          )}
          <DialogFooter>
            <div className="grow text-sm text-destructive" role="alert">
              {save.isError && <FailureNote failure={failureOf(save.error)} />}
            </div>
            <Button
              type="button"
              variant="quiet"
              disabled={pending || !complete}
              onClick={() => test.mutate({ ...form, id: config?.id })}
            >
              测试
            </Button>
            <DialogClose render={<Button variant="ghost" />}>取消</DialogClose>
            <Button type="submit" disabled={save.isPending || !complete}>
              {config ? "保存" : "添加"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** "北京时间工作日 9:00–12:00、14:00–18:00". */
function hours(peak: PeakHours) {
  return `${peak.zone}工作日 ${peak.weekdays.map(([from, until]) => `${from}:00–${until}:00`).join("、")}`;
}

const PRICES = [
  ["input", "输入", "选填"],
  // Short enough for the narrowest phone, where the three fields share a line.
  ["cache", "缓存命中", "同输入"],
  ["output", "输出", "选填"],
] as const;

/** A model's prices for input, input read from the provider's cache, and output. */
function PriceFields({
  id,
  label,
  prices,
  onChange,
}: {
  id: string;
  /** Of a group of prices, where there are several. */
  label?: string;
  prices: ModelPrices;
  onChange: (prices: ModelPrices) => void;
}) {
  return (
    <div role="group" aria-label={label} className="grid grid-cols-3 gap-3">
      {PRICES.map(([key, name, placeholder]) => (
        <Field key={key} label={name} id={`${id}-${key}`}>
          <NumberField
            id={`${id}-${key}`}
            min={0}
            decimals={6}
            placeholder={placeholder}
            value={prices[key]}
            onValueChange={(price) => onChange({ ...prices, [key]: price })}
          />
        </Field>
      ))}
    </div>
  );
}
