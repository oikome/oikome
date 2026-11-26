// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Drawing a chart's result with ECharts (Apache-2.0). Loaded on demand, when
// an Analysis view first shows a chart. Only the pieces used are bundled
// (bars, lines, pie, treemap; axes, legend, tooltip, zooming along the axis;
// the canvas renderer). It
// draws what the node computed and nothing else: no data is fetched or
// totalled here. ECharts needs no script from strings, no WebAssembly and no
// workers, so it runs under the app's own policy.

import * as echarts from "echarts/core";
import { BarChart, LineChart, PieChart, TreemapChart } from "echarts/charts";
import { DataZoomComponent, GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([BarChart, LineChart, PieChart, TreemapChart, DataZoomComponent, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer]);

/** More groups than this along the axis, and a slider under it shows which part is seen (always on the whole screen). */
const ZOOMED = 40;
/** Columns and bars seen at first when there are more: enough to tell apart. */
const SEEN = 120;

/** What to draw: the groups along the axis, and a value of each for every series. */
export interface Drawing {
  kind: "column" | "bar" | "line" | "area" | "pie" | "treemap";
  groups: string[];
  series: { name: string; values: (number | null)[] }[];
  /** Whether the groups are in time's order: then the newest are seen first when not all fit. */
  newest?: boolean;
  /** Whether the chart fills the screen: then the wheel zooms, and both axes have a slider. */
  full?: boolean;
  /** What the values are, for the value axis, when there is no legend. */
  valueName?: string;
  /** A number in the reader's words. */
  format: (value: number) => string;
  /** A group clicked (its place in `groups`), and where, in the window's coordinates. */
  onPick?: (group: number, at: { x: number; y: number }) => void;
}

/** Ink: every series is ink, told apart by its strength. Bold: the five tones, then ink. */
const PALETTES: Record<string, string[]> = {
  ink: ["#141310", "#8b8a86", "#c9c8c4", "#4a4944", "#e2e1dd", "#6f6e6a", "#a7a6a2", "#2f2e2a"],
  bold: ["#ff3d81", "#3ddc97", "#ffd23f", "#c8a2ff", "#ff6b5e", "#111111"],
};

function option(drawing: Drawing, host: HTMLElement): echarts.EChartsCoreOption {
  const style = getComputedStyle(host);
  const token = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  const theme = document.documentElement.dataset.theme === "bold" ? "bold" : "ink";
  const ink = token("--ink", "#141310");
  const soft = theme === "bold" ? "#5c5c5c" : "#6f6e6a";
  const rule = theme === "bold" ? "#111111" : "#dcdbd8";
  const faint = theme === "bold" ? "#e9e2d2" : "#eeedea";
  const many = drawing.series.length > 1;
  const text = { fontFamily: style.fontFamily, color: soft, fontSize: 12 };
  const common = {
    color: PALETTES[theme],
    textStyle: text,
    animationDuration: 240,
    legend: { show: many, type: "scroll", top: 0, left: 0, icon: "rect", itemWidth: 10, itemHeight: 10, textStyle: { ...text, color: ink } },
  };
  const tooltip = {
    confine: true,
    backgroundColor: token("--raised", "#ffffff"),
    borderColor: rule,
    textStyle: { ...text, color: ink },
    valueFormatter: (value: unknown) => (typeof value === "number" ? drawing.format(value) : "–"),
  };
  // A pie: each group a slice of the first series, its share told.
  if (drawing.kind === "pie") {
    const first = drawing.series[0];
    return {
      ...common,
      legend: { show: drawing.groups.length > 1, type: "scroll", orient: "vertical", right: 0, top: "middle", icon: "rect", itemWidth: 10, itemHeight: 10, textStyle: { ...text, color: ink } },
      tooltip: { ...tooltip, trigger: "item" },
      series: [
        {
          type: "pie",
          name: first?.name,
          radius: ["0%", "72%"],
          center: ["42%", "52%"],
          cursor: drawing.onPick ? "pointer" : "default",
          itemStyle: { borderColor: token("--raised", "#ffffff"), borderWidth: theme === "bold" ? 2 : 1 },
          label: { fontFamily: style.fontFamily, fontSize: 12, color: ink, formatter: (p: { name: string; percent: number }) => `${p.name}\n${p.percent.toFixed(1)}%` },
          data: drawing.groups
            .map((name, i) => ({ name, value: first?.values[i] ?? 0, group: i }))
            .filter((item) => item.value > 0),
        },
      ],
    };
  }
  if (drawing.kind === "treemap") {
    const first = drawing.series[0];
    return {
      ...common,
      legend: { show: false },
      tooltip: { ...tooltip, trigger: "item" },
      series: [
        {
          type: "treemap",
          left: 0,
          top: 0,
          right: 0,
          bottom: 0,
          roam: false,
          nodeClick: false,
          breadcrumb: { show: false },
          itemStyle: { borderColor: token("--raised", "#ffffff"), borderWidth: 2, gapWidth: 2 },
          label: { fontFamily: style.fontFamily, fontSize: 12, formatter: (p: { name: string; value: number }) => `${p.name}\n${drawing.format(p.value)}` },
          data: drawing.groups
            .map((name, i) => ({ name, value: Math.max(first?.values[i] ?? 0, 0), group: i }))
            .filter((item) => item.value > 0),
        },
      ],
    };
  }
  const across = drawing.kind === "bar";
  const groups = {
    type: "category",
    data: drawing.groups,
    inverse: across,
    axisTick: { show: false },
    axisLine: { lineStyle: { color: rule } },
    axisLabel: { ...text, hideOverlap: true },
  };
  // Counts are whole: no ticks between them.
  const whole = drawing.series.every((series) => series.values.every((v) => v === null || Number.isInteger(v)));
  const values = {
    type: "value",
    minInterval: whole ? 1 : undefined,
    name: drawing.valueName,
    nameLocation: "end",
    nameTextStyle: { ...text, align: across ? "right" : "left", padding: across ? [0, 0, 0, 0] : [0, 0, 4, 0] },
    axisLabel: { ...text, formatter: (value: number) => drawing.format(value) },
    splitLine: { lineStyle: { color: faint } },
  };
  const lines = drawing.kind === "line" || drawing.kind === "area";
  // Any chart along an axis zooms: by dragging it pans, and with Ctrl and
  // the wheel (a trackpad's pinch) it zooms, so the page still scrolls with
  // the wheel alone; on the whole screen the wheel zooms by itself. A slider
  // under the axis (beside it for bars) shows the part seen when there are
  // many groups, and always on the whole screen, with one for the values too.
  const full = Boolean(drawing.full);
  const zoomed = drawing.groups.length > ZOOMED || full;
  // Columns and bars too many to tell apart open on a part of them: the
  // newest in time's order, else the first; lines show all.
  const count = drawing.groups.length;
  const part = !lines && count > SEEN ? (drawing.newest ? { startValue: count - SEEN, endValue: count - 1 } : { startValue: 0, endValue: SEEN - 1 }) : {};
  const axis = { ...(across ? { yAxisIndex: 0 } : { xAxisIndex: 0 }), ...part };
  const slider = {
    type: "slider",
    borderColor: rule,
    backgroundColor: "transparent",
    fillerColor: theme === "bold" ? "rgba(17,17,17,0.08)" : "rgba(20,19,16,0.06)",
    dataBackground: { lineStyle: { color: rule }, areaStyle: { color: faint } },
    handleStyle: { color: token("--raised", "#ffffff"), borderColor: soft },
    moveHandleSize: 4,
    showDetail: false,
    brushSelect: false,
  };
  const dataZoom =
    drawing.groups.length > 1
      ? [
          { type: "inside", ...axis, zoomOnMouseWheel: full ? true : "ctrl", moveOnMouseWheel: false, moveOnMouseMove: true },
          ...(zoomed ? [{ ...slider, ...axis, ...(across ? { orient: "vertical", right: 4, width: 16 } : { bottom: 4, height: 16 }) }] : []),
          // The values' scale, on the whole screen: only the axis changes, every group stays.
          ...(full
            ? [{ ...slider, ...(across ? { xAxisIndex: 0, bottom: 4, height: 16 } : { yAxisIndex: 0, orient: "vertical", left: 4, width: 16 }), filterMode: "none" }]
            : []),
        ]
      : undefined;
  return {
    ...common,
    tooltip: { ...tooltip, trigger: "axis", axisPointer: { type: lines ? "line" : "shadow" } },
    // The legend on top, and under it the name of the value axis: each a line of its own.
    grid: {
      left: full && !across ? 32 : 4,
      top: many && drawing.valueName && !across ? 60 : many || (drawing.valueName && !across) ? 36 : 12,
      bottom: (across && drawing.valueName ? 24 : 4) + ((zoomed && !across) || (full && across) ? 28 : 0),
      right: zoomed && across ? 40 : 16,
      containLabel: true,
    },
    dataZoom,
    xAxis: across ? values : groups,
    yAxis: across ? groups : values,
    series: drawing.series.map((series) => ({
      name: series.name,
      type: lines ? "line" : "bar",
      data: series.values,
      ...(lines
        ? { showSymbol: drawing.groups.length <= 31, symbolSize: 5, lineStyle: { width: 2 }, ...(drawing.kind === "area" ? { areaStyle: { opacity: 0.14 } } : {}) }
        : { barMaxWidth: 44, ...(theme === "bold" ? { itemStyle: { borderColor: "#111111", borderWidth: 1.5 } } : {}) }),
    })),
  };
}

/** A chart in `host`, redrawn with each drawing and when the host changes size. */
export function mount(host: HTMLElement): { draw(drawing: Drawing): void; dispose(): void } {
  const chart = echarts.init(host);
  const sized = new ResizeObserver(() => chart.resize());
  sized.observe(host);
  let shown: Drawing | undefined;
  // A click on a slice or a tile: the group it is. On columns, bars and
  // lines, anywhere in a group's band (where its tooltip shows), not only on
  // the bar itself.
  const point = (event: unknown) => {
    const e = event as { clientX?: number; clientY?: number } | undefined;
    return { x: e?.clientX ?? 0, y: e?.clientY ?? 0 };
  };
  chart.on("click", (params) => {
    const data = params.data as { group?: number } | undefined;
    if (typeof data?.group === "number") shown?.onPick?.(data.group, point(params.event?.event));
  });
  chart.getZr().on("click", (event) => {
    const kind = shown?.kind;
    if (!shown?.onPick || kind === "pie" || kind === "treemap") return;
    const at = [event.offsetX, event.offsetY];
    if (!chart.containPixel({ gridIndex: 0 }, at)) return;
    const [x, y] = chart.convertFromPixel({ gridIndex: 0 }, at) as number[];
    const group = Math.round(kind === "bar" ? (y ?? -1) : (x ?? -1));
    if (group >= 0 && group < shown.groups.length) shown.onPick(group, point(event.event));
  });
  // A hand over what drills in.
  chart.getZr().on("mousemove", (event) => {
    if (shown?.kind === "pie" || shown?.kind === "treemap") return;
    const over = Boolean(shown?.onPick) && chart.containPixel({ gridIndex: 0 }, [event.offsetX, event.offsetY]);
    chart.getZr().setCursorStyle(over ? "pointer" : "default");
  });
  return {
    draw: (drawing) => {
      shown = drawing;
      chart.setOption(option(drawing, host), true);
      host.style.cursor = "";
    },
    dispose: () => {
      sized.disconnect();
      chart.dispose();
    },
  };
}
