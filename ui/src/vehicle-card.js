// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlCarImage } from "./car-image.js";
import { TlElement, contextOf, h, readData } from "./dom.js";
import { card, sheet } from "./styles.js";
import { scheduleFrom, statusFrom } from "./status.js";

function temperature(fmt, value) {
  if (value == null || !Number.isFinite(Number(value))) return null;
  return `${fmt.number(Number(value), 1)}${fmt.sep}°C`;
}

function factsFrom(data, fmt) {
  if (Array.isArray(data.facts)) return data.facts;
  const fr = fmt.locale.toLowerCase().startsWith("fr");
  const facts = [];
  if (typeof data.locked === "boolean") {
    facts.push({
      label: fr ? "Portes" : "Doors",
      value: data.locked ? (fr ? "Verrouillées" : "Locked") : (fr ? "Déverrouillées" : "Unlocked"),
    });
  }
  const inside = temperature(fmt, data.inside_temp_c);
  if (inside) facts.push({ label: fr ? "Intérieur" : "Inside", value: inside });
  const outside = temperature(fmt, data.outside_temp_c);
  if (outside) facts.push({ label: fr ? "Extérieur" : "Outside", value: outside });
  if (data.odometer_km != null) {
    facts.push({ label: fr ? "Compteur" : "Odometer", value: fmt.unit(data.odometer_km, 0, fmt.strings.km) });
  }
  if (data.sentry_mode === true) facts.push({ label: fr ? "Sentinelle" : "Sentry", value: fr ? "Active" : "On" });
  if (data.software_version) facts.push({ label: fr ? "Logiciel" : "Software", value: String(data.software_version) });
  if (data.charge_status === "unknown" || data.charge_status_stale === true) {
    facts.push({ label: "Charge", value: fr ? "Inconnue" : "Unknown" });
  }
  if (data.data_source === "telemetry" || data.data_source === "polling") {
    const telemetry = data.data_source === "telemetry";
    facts.push({
      label: fr ? "Données" : "Data",
      value: telemetry ? (fr ? "Télémétrie" : "Telemetry") : (fr ? "Interrogation" : "Polling"),
    });
  }
  return facts;
}

export class TlVehicleCard extends TlElement {
  constructor() {
    super(sheet(card));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "image-url", "alt", "unavailable"];
  }

  set imageUrl(value) {
    if (value) this.setAttribute("image-url", value);
    else this.removeAttribute("image-url");
  }

  get imageUrl() {
    return this.getAttribute("image-url");
  }

  paint(root) {
    const fmt = contextOf(this);
    const data = readData(this) || {};
    const status = statusFrom(data, fmt.strings);
    const head = h("div", { class: "hero-head" }, root);
    const titles = h("div", null, head);
    const eyebrow = h("p", { class: "eyebrow" }, titles);
    const model = data.model || fmt.strings.vehicle;
    eyebrow.textContent = data.trim ? `${model} · ${data.trim}` : model;
    h("h2", { class: "hero-name", id: "hero-name", text: data.name || fmt.strings.vehicle }, titles);
    const statusCol = h("div", { class: "hero-status" }, head);
    const chip = h("span", { class: `chip status ${status.tone}` }, statusCol);
    h("span", { class: "status-dot", "aria-hidden": "true" }, chip);
    chip.append(document.createTextNode(status.label));
    const schedule = scheduleFrom(data, fmt.strings);
    if (schedule) h("p", { class: "hero-schedule", text: schedule }, statusCol);

    const stage = h("div", { class: "hero-car" }, root);
    const photo = new TlCarImage();
    const url = this.getAttribute("image-url") || data.image_url || "";
    if (url) photo.imageUrl = url;
    const alt = this.getAttribute("alt") || data.name || fmt.strings.vehicle;
    photo.setAttribute("alt", alt);
    const unavailable = this.getAttribute("unavailable");
    if (unavailable) photo.setAttribute("unavailable", unavailable);
    photo.setAttribute("locale", fmt.locale);
    if (this.getAttribute("theme")) photo.setAttribute("theme", this.getAttribute("theme"));
    stage.append(photo);

    const energy = h("div", { class: "hero-energy" }, root);
    const battery = h("p", { class: "hero-battery" }, energy);
    const batteryText = data.battery_text
      || (data.battery_level == null ? "—" : fmt.percent(data.battery_level));
    h("span", { class: "hero-figure", text: batteryText }, battery);
    h("span", { class: "hero-label", text: fmt.strings.battery }, battery);
    const range = h("p", { class: "hero-range" }, energy);
    const rangeText = data.range_text
      || (data.rated_range_km == null ? "—" : fmt.distance(data.rated_range_km));
    h("span", { class: "hero-figure", text: rangeText }, range);
    h("span", { class: "hero-label", text: fmt.strings.range }, range);

    const meter = document.createElement("tl-battery");
    if (data.battery_level != null) meter.level = data.battery_level;
    if (data.charge_limit_soc != null) meter.limit = data.charge_limit_soc;
    meter.charging = status.charging;
    meter.setAttribute("locale", fmt.locale);
    root.append(meter);

    if (data.note) h("p", { class: "hero-note", text: data.note }, root);
    const facts = factsFrom(data, fmt);
    if (facts.length) {
      const list = h("dl", { class: "hero-facts" }, root);
      for (const fact of facts) {
        const row = h("div", null, list);
        h("dt", { text: fact.label }, row);
        h("dd", { text: fact.value }, row);
      }
    }
  }
}
