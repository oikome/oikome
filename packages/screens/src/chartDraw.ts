// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Drawing a chart's result with ECharts (Apache-2.0). Loaded on demand, when
// an Analysis view first shows a chart. Only the pieces used are bundled
// (bars, lines, treemap; axes, legend, tooltip; the canvas renderer). It
// draws what the node computed and nothing else: no data is fetched or
// totalled here. ECharts needs no script from strings, no WebAssembly and no
// workers, so it runs under the app's own policy.

import * as echarts from "echarts/core";
import { BarChart, LineChart, TreemapChart } from "echarts/charts";
import { GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([BarChart, LineChart, TreemapChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer]);

/** What to draw: the groups along the axis, and a value of each for every series. */
export interface Drawing {
  kind: "column" | "bar" | "line" | "area" | "treemap";
  groups: string[];
  series: { name: string; values: (number | null)[] }[];
  /** A number in the reader's words. */
  format: (value: number) => string;
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
            .map((name, i) => ({ name, value: Math.max(first?.values[i] ?? 0, 0) }))
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
  const values = {
    type: "value",
    axisLabel: { ...text, formatter: (value: number) => drawing.format(value) },
    splitLine: { lineStyle: { color: faint } },
  };
  const lines = drawing.kind === "line" || drawing.kind === "area";
  return {
    ...common,
    tooltip: { ...tooltip, trigger: "axis", axisPointer: { type: lines ? "line" : "shadow" } },
    grid: { left: 4, right: 16, top: many ? 36 : 12, bottom: 4, containLabel: true },
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
  return {
    draw: (drawing) => chart.setOption(option(drawing, host), true),
    dispose: () => {
      sized.disconnect();
      chart.dispose();
    },
  };
}
