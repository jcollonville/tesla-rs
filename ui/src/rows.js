// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlElement, contextOf, emit, h, nowOf, readData, svgEl } from "./dom.js";
import { efficiency, place } from "./format.js";
import { day, recent, trip, sheet } from "./styles.js";

function batterySpan(fmt, start, end) {
  if (start != null && end != null) return `${fmt.number(Number(start), 0)} → ${fmt.percent(end)}`;
  if (start != null) return fmt.percent(start);
  return "—";
}

function socBar(start, end) {
  if (start == null || end == null) return null;
  const low = Math.min(Number(start), Number(end));
  const high = Math.max(Number(start), Number(end));
  const board = svgEl("svg", {
    class: end >= start ? "soc-bar gain" : "soc-bar use",
    viewBox: "0 0 100 6",
    preserveAspectRatio: "none",
    "aria-hidden": "true",
    focusable: "false",
  });
  svgEl("rect", { class: "soc-track", x: "0", y: "0", width: "100", height: "6", rx: "3" }, board);
  svgEl("rect", {
    class: "soc-fill",
    x: Math.max(0, Math.min(100, low)).toFixed(1),
    y: "0",
    width: Math.max(1.5, high - low).toFixed(1),
    height: "6",
    rx: "3",
  }, board);
  return board;
}

function times(parent, data, fmt) {
  const box = h("span", { class: "trip-times" }, parent);
  h("time", { datetime: data.start_at || "", text: data.start_at ? fmt.clock(data.start_at) : "—" }, box);
  if (data.end_at) h("span", { text: fmt.clock(data.end_at) }, box);
  else {
    const live = h("span", { class: "live" }, box);
    h("span", { class: "live-dot" }, live);
    live.append(document.createTextNode(fmt.strings.inProgress));
  }
  return box;
}

function sourceChip(parent, source) {
  if (!source || source === "poll" || source === "stream") return;
  h("span", { class: "chip source", text: source }, parent);
}

function rowShell(href) {
  if (href) return h("a", { class: "trip", href });
  const row = h("div", { class: "trip" });
  row.tabIndex = 0;
  row.setAttribute("role", "link");
  return row;
}

export class TlDriveRow extends TlElement {
  constructor() {
    super(sheet(trip));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "href"];
  }

  set href(value) {
    if (value) this.setAttribute("href", value);
    else this.removeAttribute("href");
  }

  get href() {
    return this.getAttribute("href");
  }

  paint(root) {
    const fmt = contextOf(this);
    const data = readData(this) || {};
    const href = this.getAttribute("href") || data.href || "";
    const from = place(data.start_address, data.start_geofence);
    const to = place(data.end_address, data.end_geofence);
    const fromName = from ? from.name : fmt.strings.unknownPlace;
    const toName = data.end_at ? (to ? to.name : fmt.strings.unknownPlace) : fmt.strings.enRoute;
    const row = rowShell(href);
    row.classList.add("trip");
    const when = data.start_at ? fmt.dayLabel(data.start_at, nowOf(this)) : "";
    const clock = data.start_at ? fmt.clock(data.start_at) : "";
    row.setAttribute("aria-label", `${when} ${clock}, ${fromName}, ${toName}, ${fmt.distance(data.distance_km)}`.trim());
    times(row, data, fmt);
    const route = h("span", { class: "route" }, row);
    const start = h("span", { class: "route-stop start", text: fromName }, route);
    if (from?.detail) start.title = from.detail;
    const end = h("span", { class: "route-stop end", text: toName }, route);
    if (to?.detail && data.end_at) end.title = to.detail;
    const figures = h("span", { class: "trip-figures" }, row);
    h("span", { class: "figure", text: fmt.distance(data.distance_km) }, figures);
    h("span", { class: "muted", text: fmt.duration(data.duration_s) }, figures);
    const energy = data.energy_kwh != null ? data.energy_kwh : data.energy_used_kwh;
    const estimated = data.energy_kwh != null && data.energy_used_kwh == null;
    let energyText = energy == null ? "—" : fmt.energy(energy);
    if (estimated && energy != null) energyText = `≈${energyText}`;
    const wh = efficiency(Number(energy), Number(data.distance_km));
    const stats = h("span", { class: "trip-stats" }, row);
    const energyStat = h("span", null, stats);
    h("span", { class: "stat-label", text: fmt.strings.energy }, energyStat);
    energyStat.append(document.createTextNode(energyText));
    const eff = h("span", null, stats);
    h("span", { class: "stat-label", text: fmt.strings.efficiency }, eff);
    eff.append(document.createTextNode(wh == null ? "—" : fmt.unit(wh, 0, "Wh/km")));
    const battery = h("span", null, stats);
    h("span", { class: "stat-label", text: fmt.strings.battery }, battery);
    battery.append(document.createTextNode(batterySpan(fmt, data.start_battery_level, data.end_battery_level)));
    sourceChip(row, data.source);
    if (!href) {
      row.addEventListener("click", () => emit(this, "tl-open", { id: data.id }));
      row.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          emit(this, "tl-open", { id: data.id });
        }
      });
    }
    root.append(row);
  }
}

export class TlChargeRow extends TlElement {
  constructor() {
    super(sheet(trip));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "href"];
  }

  set href(value) {
    if (value) this.setAttribute("href", value);
    else this.removeAttribute("href");
  }

  get href() {
    return this.getAttribute("href");
  }

  paint(root) {
    const fmt = contextOf(this);
    const data = readData(this) || {};
    const href = this.getAttribute("href") || data.href || "";
    const where = place(data.address, data.geofence);
    const name = where ? where.name : fmt.strings.unknownPlace;
    const dc = data.is_dc === true;
    const added = data.energy_added_kwh;
    const figure = added == null ? fmt.strings.inProgress.replace(/^./, (ch) => ch.toLocaleUpperCase(fmt.locale)) : `+${fmt.energy(added)}`;
    const row = rowShell(href);
    row.classList.add("charge");
    if (dc) row.classList.add("dc");
    row.setAttribute("aria-label", `${data.start_at ? fmt.clock(data.start_at) : ""} ${name} ${figure}`.trim());
    times(row, data, fmt);
    const route = h("span", { class: "route single" }, row);
    const stop = h("span", { class: "route-stop charge-stop", text: name }, route);
    if (where?.detail) stop.title = where.detail;
    const kind = h("span", { class: "muted kind" }, route);
    if (dc) h("span", { class: "chip dc", text: "DC" }, kind);
    const charger = data.charger_type || (data.is_dc === false ? "AC" : data.is_dc == null ? fmt.strings.charging : "");
    if (charger) kind.append(document.createTextNode(dc ? ` ${charger}` : charger));
    const figures = h("span", { class: "trip-figures" }, row);
    h("span", { class: "figure", text: figure }, figures);
    const seconds = data.duration_s ?? (data.start_at ? Math.max(0, Math.round(((data.end_at ? new Date(data.end_at) : new Date(nowOf(this))) - new Date(data.start_at)) / 1000)) : null);
    h("span", { class: "muted", text: fmt.duration(seconds) }, figures);
    const stats = h("span", { class: "trip-stats" }, row);
    const battery = h("span", { class: "soc" }, stats);
    h("span", { class: "stat-label", text: fmt.strings.battery }, battery);
    battery.append(document.createTextNode(batterySpan(fmt, data.start_battery_level, data.end_battery_level)));
    const bar = socBar(data.start_battery_level, data.end_battery_level);
    if (bar) battery.append(bar);
    const power = h("span", null, stats);
    h("span", { class: "stat-label", text: fmt.strings.maxPower }, power);
    power.append(document.createTextNode(fmt.unit(data.max_power_kw, 0, "kW")));
    const cost = h("span", null, stats);
    h("span", { class: "stat-label", text: fmt.strings.cost }, cost);
    cost.append(document.createTextNode(fmt.money(data.cost)));
    sourceChip(row, data.source);
    if (!href) row.addEventListener("click", () => emit(this, "tl-open", { id: data.id }));
    root.append(row);
  }
}

export class TlDayGroup extends TlElement {
  constructor() {
    super(sheet(day));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "heading", "summary"];
  }

  paint(root) {
    const heading = this.getAttribute("heading") || "";
    const section = h("section", { class: "day", "aria-label": heading }, root);
    const header = h("header", { class: "day-head" }, section);
    h("h2", { text: heading }, header);
    h("p", { class: "muted", text: this.getAttribute("summary") || "" }, header);
    const list = h("div", { class: "trips", role: "list" }, section);
    list.append(document.createElement("slot"));
  }
}

function recentView(data, fmt, now) {
  if (!data) return null;
  if (data.when || data.primary) return data;
  if (!data.start_at) return null;
  const open = !data.end_at;
  if (data.energy_added_kwh != null || data.is_dc != null || data.charger_type) {
    const where = place(data.address, data.geofence);
    return {
      href: data.href || "",
      when: `${fmt.dayLabel(data.start_at, now)} · ${fmt.clock(data.start_at)}`,
      from: null,
      to: where ? where.name : fmt.strings.unknownPlace,
      primary: data.energy_added_kwh == null ? "—" : `+${fmt.energy(data.energy_added_kwh)}`,
      secondary: fmt.duration(data.duration_s),
      open,
    };
  }
  const from = place(data.start_address, data.start_geofence);
  const to = place(data.end_address, data.end_geofence);
  const wh = efficiency(Number(data.energy_used_kwh), Number(data.distance_km));
  const secondary = [fmt.duration(data.duration_s)];
  if (wh != null) secondary.push(fmt.unit(wh, 0, "Wh/km"));
  return {
    href: data.href || "",
    when: `${fmt.dayLabel(data.start_at, now)} · ${fmt.clock(data.start_at)}`,
    from: from ? from.name : fmt.strings.unknownPlace,
    to: open ? fmt.strings.enRoute : (to ? to.name : fmt.strings.unknownPlace),
    primary: fmt.distance(data.distance_km),
    secondary: secondary.join(" · "),
    open,
  };
}

export class TlRecentCard extends TlElement {
  constructor() {
    super(sheet(recent));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "title", "empty", "link-href", "link-label"];
  }

  paint(root) {
    const fmt = contextOf(this);
    const view = recentView(readData(this), fmt, nowOf(this));
    const head = h("div", { class: "card-head" }, root);
    h("h2", { text: this.getAttribute("title") || "" }, head);
    const linkHref = this.getAttribute("link-href");
    const linkLabel = this.getAttribute("link-label");
    if (linkHref && linkLabel) h("a", { class: "text-link", href: linkHref, text: linkLabel }, head);
    if (!view) {
      h("p", { class: "muted", text: this.getAttribute("empty") || "" }, root);
      return;
    }
    const link = h("a", { class: "recent-link", href: view.href || "#" }, root);
    const when = h("p", { class: "recent-when" }, link);
    when.append(document.createTextNode(view.when || ""));
    if (view.open) {
      when.append(document.createTextNode(" · "));
      h("span", { class: "live-dot" }, when);
      when.append(document.createTextNode(` ${fmt.strings.inProgress}`));
    }
    const route = h("div", { class: view.from ? "route" : "route single" }, link);
    if (view.from) h("span", { class: "route-stop start", text: view.from }, route);
    h("span", { class: "route-stop end", text: view.to || "" }, route);
    const metrics = h("p", { class: "recent-metrics" }, link);
    h("span", { class: "figure", text: view.primary || "" }, metrics);
    h("span", { class: "muted", text: view.secondary || "" }, metrics);
  }
}
