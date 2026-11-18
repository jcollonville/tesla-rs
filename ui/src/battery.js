// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlElement, contextOf, svgEl } from "./dom.js";
import { battery, sheet } from "./styles.js";

export class TlBattery extends TlElement {
  constructor() {
    super(sheet(battery));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "level", "limit", "charging", "low-at"];
  }

  set level(value) {
    this.#setNum("level", value);
  }

  get level() {
    return this.#num("level");
  }

  set limit(value) {
    this.#setNum("limit", value);
  }

  get limit() {
    return this.#num("limit");
  }

  set charging(value) {
    if (value) this.setAttribute("charging", "");
    else this.removeAttribute("charging");
  }

  get charging() {
    return this.hasAttribute("charging");
  }

  #setNum(name, value) {
    if (value == null || value === "" || Number.isNaN(Number(value))) this.removeAttribute(name);
    else this.setAttribute(name, String(value));
  }

  #num(name) {
    const raw = this.getAttribute(name);
    if (raw == null || raw === "") return null;
    const value = Number(raw);
    return Number.isFinite(value) ? value : null;
  }

  paint(root) {
    const fmt = contextOf(this);
    const level = this.level;
    if (level == null) return;
    const pct = Math.min(100, Math.max(0, level));
    const charging = this.charging;
    const lowAt = this.#num("low-at") ?? 20;
    let tone = "battery-meter";
    if (charging) tone += " charging";
    else if (pct <= lowAt) tone += " low";
    const meter = svgEl("svg", {
      class: tone,
      viewBox: "0 0 100 8",
      preserveAspectRatio: "none",
      role: "img",
      "aria-label": `${fmt.strings.battery} ${Math.round(pct)} %`,
      focusable: "false",
    }, root);
    svgEl("rect", { class: "battery-track", width: "100", height: "8", rx: "4" }, meter);
    svgEl("rect", {
      class: "battery-fill",
      width: Math.max(2, pct).toFixed(1),
      height: "8",
      rx: "4",
    }, meter);
    const limit = this.limit;
    if (limit != null && limit > 0 && limit < 100) {
      svgEl("rect", {
        class: "battery-limit",
        x: (limit - 0.4).toFixed(1),
        y: "0",
        width: "0.8",
        height: "8",
      }, meter);
    }
  }
}
