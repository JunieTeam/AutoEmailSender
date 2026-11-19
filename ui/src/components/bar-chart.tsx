import { Bar, BarChart as Chart, CartesianGrid, Tooltip, type TooltipContentProps, XAxis, YAxis } from "recharts";

export type Series = { name: string; unit: string; color: string };
/** A group of columns: its axis label, its full name for the tooltip, one value per series. */
export type Point = { label: string; title: string; values: readonly number[] };

const AXIS_TEXT = { fill: "var(--muted-foreground)", fontSize: 11 };

/**
 * Grouped columns over time (docs/ui.md#图表): hairline gridlines, columns rounded at the top and
 * apart by a 2px gap, a band and the numbers on the group under the pointer or keyboard focus.
 */
export function BarChart({ series, points, caption }: { series: readonly Series[]; points: Point[]; caption: string }) {
  return (
    <Chart
      data={points}
      responsive
      style={{ width: "100%", height: 200 }}
      margin={{ top: 4, right: 0, bottom: 0, left: 0 }}
      // Dense, as 90 days on a phone, the gaps shrink with the columns instead of squeezing them out.
      barGap={points.length > 31 ? "10%" : 2}
      barCategoryGap={points.length > 31 ? "10%" : "20%"}
      title={caption}
    >
      <CartesianGrid vertical={false} stroke="var(--border)" />
      <XAxis
        dataKey="label"
        tick={AXIS_TEXT}
        tickLine={false}
        axisLine={{ stroke: "var(--border)" }}
        tickMargin={8}
        minTickGap={20}
        interval="preserveStartEnd"
      />
      <YAxis allowDecimals={false} tick={AXIS_TEXT} tickLine={false} axisLine={false} width="auto" tickCount={4} />
      <Tooltip
        cursor={{ fill: "var(--muted)" }}
        isAnimationActive={false}
        content={<Tip series={series} points={points} />}
      />
      {series.map((s, i) => (
        <Bar
          key={s.name}
          name={s.name}
          dataKey={(p: Point) => p.values[i]}
          fill={s.color}
          radius={[4, 4, 0, 0]}
          maxBarSize={14}
        />
      ))}
    </Chart>
  );
}

/** Given `active` and `activeIndex` by Recharts, which clones it into the tooltip. */
function Tip({
  active,
  activeIndex,
  series,
  points,
}: Partial<Pick<TooltipContentProps, "active" | "activeIndex">> & { series: readonly Series[]; points: Point[] }) {
  const point = activeIndex == null ? undefined : points[Number(activeIndex)];
  if (!active || !point) return null;
  return (
    // Read out as the arrow keys move along the chart, as Recharts' own tooltip is.
    <div
      role="status"
      aria-live="polite"
      className="rounded-lg bg-background px-3 py-2 text-xs whitespace-nowrap shadow-[0_4px_16px_var(--pop-shadow)] ring-1 ring-border"
    >
      <p className="mb-1 font-medium">{point.title}</p>
      {series.map((s, i) => (
        <p key={s.name} className="flex items-center gap-1.5 text-muted-foreground">
          <span className="size-2 rounded-full" style={{ background: s.color }} />
          {s.name} <span className="ml-auto pl-3 text-foreground tabular-nums">{point.values[i]}</span> {s.unit}
        </p>
      ))}
    </div>
  );
}

export function Legend({ series }: { series: readonly Series[] }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      {series.map((s) => (
        <span key={s.name} className="flex items-center gap-1.5">
          <span className="size-2 rounded-full" style={{ background: s.color }} />
          {s.name}
        </span>
      ))}
    </div>
  );
}
