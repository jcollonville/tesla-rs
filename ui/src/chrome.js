// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlElement, contextOf, emit, h, readData } from "./dom.js";
import { efficiency } from "./format.js";
import { chips, sheet, summary, tabs } from "./styles.js";

function statsItems(stats, fmt) {
  const wh = efficiency(Number(stats.energy_used_kwh), Number(stats.distance_km));
  return [
    { label: fmt.strings.drives, value: fmt.number(Number(stats.drive_count) || 0, 0) },
    { label: fmt.strings.distance, value: fmt.distance(stats.distance_km) },
    { label: fmt.strings.behindWheel, value: fmt.duration(stats.drive_duration_s) },
    { label: fmt.strings.energy, value: fmt.energy(stats.energy_used_kwh) },
    { label: fmt.strings.efficiency, value: wh == null ? "—" : fmt.unit(wh, 0, "Wh/km") },
  ];
}

export class TlSummaryStrip extends TlElement {
  constructor() {
    super(sheet(summary));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "variant", "title", "label", "link-href", "link-label"];
  }

  paint(root) {
    const fmt = contextOf(this);
    let items = readData(this);
    if (items && !Array.isArray(items) && items.drive_count != null) items = statsItems(items, fmt);
    if (!Array.isArray(items)) items = [];
    const variant = this.getAttribute("variant") || "strip";
    if (variant === "card") {
      const head = h("div", { class: "card-head" }, root);
      h("h2", { text: this.getAttribute("title") || "" }, head);
      const href = this.getAttribute("link-href");
      const label = this.getAttribute("link-label");
      if (href && label) h("a", { class: "text-link", href, text: label }, head);
    }
    const list = h("dl", {
      class: variant === "strip" ? "metrics summary" : "metrics",
      "aria-label": this.getAttribute("label") || this.getAttribute("title") || "",
    }, root);
    for (const item of items) {
      const row = h("div", null, list);
      h("dt", { text: item.label }, row);
      h("dd", { text: item.value }, row);
    }
  }
}

export class TlPeriodChips extends TlElement {
  constructor() {
    super(sheet(chips));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "label"];
  }

  paint(root) {
    const label = this.getAttribute("label") || "";
    if (!this.hasAttribute("role")) this.setAttribute("role", "navigation");
    if (label && this.getAttribute("aria-label") !== label) this.setAttribute("aria-label", label);
    const options = readData(this) || [];
    for (const option of options) {
      const link = h("a", { href: option.href || "#", text: option.label || "" }, root);
      if (option.current) link.setAttribute("aria-current", "page");
      link.addEventListener("click", (event) => {
        emit(this, "tl-change", { id: option.id, href: option.href || "" });
        if (!option.href) event.preventDefault();
      });
    }
  }
}

export class TlTabs extends TlElement {
  constructor() {
    super(sheet(tabs));
    this.addEventListener("keydown", (event) => {
      const tabs = [...this.shadowRoot.querySelectorAll("[role='tab']")];
      const index = tabs.indexOf(this.shadowRoot.activeElement);
      if (index < 0) return;
      let next = null;
      if (event.key === "ArrowRight") next = tabs[(index + 1) % tabs.length];
      if (event.key === "ArrowLeft") next = tabs[(index - 1 + tabs.length) % tabs.length];
      if (event.key === "Home") next = tabs[0];
      if (event.key === "End") next = tabs[tabs.length - 1];
      if (!next) return;
      event.preventDefault();
      next.focus();
    });
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "label"];
  }

  paint(root) {
    const label = this.getAttribute("label") || "";
    if (this.getAttribute("role") !== "tablist") this.setAttribute("role", "tablist");
    if (label && this.getAttribute("aria-label") !== label) this.setAttribute("aria-label", label);
    const tabs = readData(this) || [];
    for (const tab of tabs) {
      const link = h("a", {
        role: "tab",
        id: tab.id || "",
        href: tab.href || "#",
        text: tab.label || "",
      }, root);
      link.setAttribute("aria-selected", tab.selected ? "true" : "false");
      if (tab.selected) link.setAttribute("aria-current", "page");
      link.addEventListener("click", () => emit(this, "tl-change", { id: tab.id, href: tab.href || "" }));
    }
  }
}
