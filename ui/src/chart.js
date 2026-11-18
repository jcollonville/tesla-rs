// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountChart } from "./chart-engine.js";
import { TlElement, h, readData, svgEl } from "./dom.js";
import { chart, sheet } from "./styles.js";

function fallback(data) {
  if (!data || data.kind === "line") {
    const points = [];
    const series = data?.series?.[0];
    const xs = data?.t || [];
    if (series && xs.length > 1) {
      xs.forEach((x, index) => {
        const y = series.values[index];
        if (y != null) points.push([x, y]);
      });
    }
    if (points.length < 2) return null;
    const minX = points[0][0];
    const maxX = points[points.length - 1][0] || 1;
    const minY = Math.min(...points.map((point) => point[1]));
    const maxY = Math.max(...points.map((point) => point[1]));
    const spanX = maxX - minX || 1;
    const spanY = maxY - minY || 1;
    const coords = points.map(([x, y]) => {
      const px = ((x - minX) / spanX) * 100;
      const py = 40 - ((y - minY) / spanY) * 36;
      return `${px.toFixed(2)},${py.toFixed(2)}`;
    }).join(" ");
    const board = svgEl("svg", {
      class: "chart-fallback",
      viewBox: "0 0 100 44",
      width: "640",
      height: "180",
      preserveAspectRatio: "none",
      role: "img",
      "aria-hidden": "true",
    });
    svgEl("polyline", { fill: "none", stroke: "currentColor", "stroke-width": "1.2", points: coords }, board);
    return board;
  }
  return null;
}

export class TlChart extends TlElement {
  constructor() {
    super(sheet(chart));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "title", "summary", "variant", "sync"];
  }

  paint(root) {
    const data = this._chart();
    const title = this.getAttribute("title") || data?.title || "";
    const summary = this.getAttribute("summary") || data?.summary || "";
    const figure = h("figure", { class: "chart", tabindex: "0", "aria-label": `${title}. ${summary}` }, root);
    if (this.getAttribute("sync")) figure.setAttribute("data-sync", this.getAttribute("sync"));
    const caption = h("figcaption", null, figure);
    h("span", { class: "chart-title", text: title }, caption);
    h("span", { class: "chart-summary", text: summary }, caption);
    const plot = h("div", { class: "chart-plot" }, figure);
    const staticLine = fallback(data);
    if (staticLine) plot.append(staticLine);
    if (data) {
      mountChart(figure, data);
      this.classList.add("ready");
    }
  }

  _chart() {
    const data = readData(this);
    if (data && data.series) return data;
    const figure = this.querySelector(":scope > figure.chart");
    const raw = figure?.getAttribute("data-chart");
    if (!raw) return data;
    try {
      return JSON.parse(raw);
    } catch {
      return null;
    }
  }
}
