// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlElement, contextOf, h, readData, svgEl } from "./dom.js";
import { map, sheet } from "./styles.js";

export class TlMiniMap extends TlElement {
  constructor() {
    super(sheet(map));
    this._lat = null;
    this._lon = null;
    this._headingProp = null;
    this._tiles = false;
  }

  set latitude(value) {
    this._lat = value == null ? null : Number(value);
    if (this.isConnected && !this._rendering) this.render();
  }

  get latitude() {
    return this._lat;
  }

  set longitude(value) {
    this._lon = value == null ? null : Number(value);
    if (this.isConnected && !this._rendering) this.render();
  }

  get longitude() {
    return this._lon;
  }

  set heading(value) {
    this._headingProp = value == null ? null : Number(value);
    if (this.isConnected && !this._rendering) this.render();
  }

  get heading() {
    return this._headingProp;
  }

  paint(root) {
    const fmt = contextOf(this);
    const data = { ...(readData(this) || {}) };
    const positioned = Number.isFinite(this._lat) && Number.isFinite(this._lon);
    if (positioned && data.empty !== true && !data.roads) data.empty = false;
    const width = Number(data.width) || 360;
    const height = Number(data.height) || 220;
    const cx = width / 2;
    const cy = height / 2;
    const stack = h("div", { class: "map-stack" }, root);
    const tiles = h("div", { class: "tiles" }, stack);
    const slot = document.createElement("slot");
    slot.name = "tiles";
    tiles.append(slot);
    const hasTiles = slot.assignedNodes({ flatten: true }).some((node) => node.nodeType === 1);
    this.classList.toggle("has-tiles", hasTiles);
    if (hasTiles !== this._tiles) this._tiles = hasTiles;
    slot.addEventListener("slotchange", () => {
      const now = slot.assignedNodes({ flatten: true }).some((node) => node.nodeType === 1);
      if (now === this._tiles || this._rendering) return;
      this._tiles = now;
      this.render();
    });

    const empty = data.empty !== false && !data.roads?.length && !positioned;
    const moving = Boolean(data.moving);
    let klass = "mini-map";
    if (moving) klass += " live";
    if (empty) klass += " empty";
    const label = data.label || (empty ? fmt.strings.unknownPosition : fmt.strings.unknownPosition);
    const board = svgEl("svg", {
      class: klass,
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": label,
      focusable: "false",
    }, stack);
    const defs = svgEl("defs", null, board);
    const pattern = svgEl("pattern", { id: "dots", width: "18", height: "18", patternUnits: "userSpaceOnUse" }, defs);
    svgEl("circle", { class: "map-dot", cx: "1.5", cy: "1.5", r: "1" }, pattern);
    if (!empty) {
      const fade = svgEl("radialGradient", { id: "fade", cx: "0.5", cy: "0.5", r: "0.62" }, defs);
      svgEl("stop", { offset: "0.55", "stop-color": "#000", "stop-opacity": "0" }, fade);
      svgEl("stop", { offset: "1", "stop-color": "#000", "stop-opacity": "1" }, fade);
    }
    if (!hasTiles) {
      svgEl("rect", { class: "map-bg", width: String(width), height: String(height) }, board);
      svgEl("rect", { class: "map-dots", width: String(width), height: String(height), fill: "url(#dots)" }, board);
    }
    if (!empty) {
      for (const road of data.roads || []) {
        if (road) svgEl("path", { class: "map-road", d: road }, board);
      }
      svgEl("rect", { class: "map-vignette", width: String(width), height: String(height), fill: "url(#fade)" }, board);
    }
    svgEl("circle", { class: "map-pulse", cx: String(cx), cy: String(cy), r: "26" }, board);
    svgEl("circle", { class: "map-halo", cx: String(cx), cy: String(cy), r: "15" }, board);
    if (!empty) {
      const heading = data.heading ?? this._headingProp;
      if (heading != null && Number.isFinite(Number(heading))) {
        svgEl("path", {
          class: "map-heading",
          d: `M${cx} ${(cy - 22).toFixed(1)} l7 13 l-7 -4 l-7 4 z`,
          transform: `rotate(${Math.round(Number(heading))} ${cx} ${cy})`,
        }, board);
      }
      svgEl("circle", { class: "map-car", cx: String(cx), cy: String(cy), r: "7" }, board);
      if (data.scale_label) {
        const scale = svgEl("g", { class: "map-scale", transform: `translate(16 ${Math.round(height - 16)})` }, board);
        const bar = Number(data.scale_width) || 0;
        svgEl("path", { d: `M0 -4 V0 H${bar.toFixed(1)} V-4` }, scale);
        svgEl("text", { x: (bar + 6).toFixed(1), y: "0", text: data.scale_label }, scale);
      }
    }
  }
}
