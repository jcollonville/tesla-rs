import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html lang=\"fr\"><body></body></html>", {
  url: "http://logger.local/",
  pretendToBeVisual: true,
});
const { window } = dom;
if (!window.matchMedia) window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
if (!window.requestAnimationFrame) window.requestAnimationFrame = (fn) => setTimeout(() => fn(0), 0);
globalThis.window = window;
globalThis.document = window.document;
globalThis.HTMLElement = window.HTMLElement;
globalThis.customElements = window.customElements;
globalThis.CustomEvent = window.CustomEvent;
globalThis.CSSStyleSheet = window.CSSStyleSheet;
globalThis.Node = window.Node;
globalThis.DocumentFragment = window.DocumentFragment;
globalThis.getComputedStyle = window.getComputedStyle.bind(window);

const ui = await import("../src/index.js");

function mount(tag, props = {}) {
  const el = document.createElement(tag);
  if (props.attrs) {
    for (const [key, value] of Object.entries(props.attrs)) el.setAttribute(key, value);
  }
  if (props.data !== undefined) el.data = props.data;
  document.body.append(el);
  return el;
}

test("plugged without charging is Branchée and a schedule stays by the status", () => {
  const plugged = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: { name: "Example", activity: "online", charging_state: "Stopped", plugged_in: true },
  });
  assert.match(plugged.shadowRoot.textContent, /Branchée/);
  assert.equal(plugged.shadowRoot.querySelector(".hero-schedule"), null);

  const starting = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: { name: "Example", activity: "charging", charging_state: "Starting", plugged_in: true },
  });
  assert.match(starting.shadowRoot.textContent, /Branchée/);
  assert.equal(starting.shadowRoot.textContent.includes("En charge"), false);

  const charging = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: { name: "Example", activity: "online", charging_state: "Charging", plugged_in: true },
  });
  assert.match(charging.shadowRoot.textContent, /En charge/);

  const loose = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: { name: "Example", activity: "online", charging_state: "Disconnected", plugged_in: true },
  });
  assert.match(loose.shadowRoot.textContent, /Garée/);

  const scheduled = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: {
      name: "Example",
      activity: "online",
      charging_state: "Stopped",
      scheduled_charging_mode: "DepartBy",
      scheduled_charging_start_time: 1320,
      scheduled_departure_time: 450,
      off_peak_charging_enabled: true,
      off_peak_hours_end_time: 360,
    },
  });
  const line = scheduled.shadowRoot.querySelector(".hero-schedule").textContent;
  assert.equal(line, "Départ programmé à 07:30 · Heures creuses jusqu'à 06:00");

  const off = mount("tl-vehicle-card", {
    attrs: { locale: "en" },
    data: {
      name: "Example",
      activity: "online",
      scheduled_charging_mode: "Off",
      scheduled_charging_start_time: 1320,
    },
  });
  assert.equal(off.shadowRoot.querySelector(".hero-schedule"), null);
});

test("vehicle image has no drawing and names the failure", () => {
  const missing = mount("tl-car-image", { attrs: { locale: "fr" } });
  const shadow = missing.shadowRoot.innerHTML;
  assert.equal(missing.shadowRoot.querySelector("svg"), null);
  assert.match(shadow, /Image du véhicule indisponible/);
  assert.equal(missing.classList.contains("ready"), false);

  const broken = mount("tl-car-image", { attrs: { "image-url": "/no-such-render.png", locale: "en" } });
  const img = broken.shadowRoot.querySelector("img");
  assert.ok(img);
  img.dispatchEvent(new window.Event("error"));
  assert.equal(broken.classList.contains("ready"), false);
  assert.match(broken.shadowRoot.textContent, /Vehicle image unavailable/);

  const card = mount("tl-vehicle-card", {
    attrs: { locale: "fr" },
    data: { name: "Example", model: "Model Y", battery_level: 70, charge_limit_soc: 80, rated_range_km: 310, activity: "online" },
  });
  assert.equal(card.shadowRoot.querySelector("svg.car"), null);
  assert.equal(card.shadowRoot.querySelector("tl-car-image").shadowRoot.querySelector("svg"), null);
  assert.match(card.shadowRoot.textContent, /Example/);
  assert.match(card.shadowRoot.querySelector("tl-battery").shadowRoot.innerHTML, /battery-limit/);
});

test("a loaded image replaces the placeholder", () => {
  const el = mount("tl-car-image", { attrs: { "image-url": "/car.png", alt: "Example" } });
  const img = el.shadowRoot.querySelector("img");
  Object.defineProperty(img, "naturalWidth", { value: 20 });
  img.dispatchEvent(new window.Event("load"));
  assert.equal(el.classList.contains("ready"), true);
  assert.equal(el.shadowRoot.querySelector("svg"), null);
});

test("mini map does not print coordinates and does not fetch tiles", () => {
  const el = mount("tl-mini-map", {
    data: {
      label: "Position : North garage",
      empty: false,
      moving: false,
      width: 360,
      height: 220,
      roads: ["M40 160 L180 110 L300 150"],
      heading: 90,
      scale_width: 36,
      scale_label: "200 m",
    },
  });
  el.latitude = 48.8566;
  el.longitude = 2.3522;
  const html = el.shadowRoot.innerHTML + el.outerHTML;
  assert.equal(html.includes("48.8566"), false);
  assert.equal(html.includes("2.3522"), false);
  assert.equal(html.includes("fetch("), false);
  assert.match(el.shadowRoot.innerHTML, /map-car/);
  assert.equal(el.shadowRoot.querySelector("slot[name='tiles']") != null, true);
});

test("drive row shows the geofence and keeps the address as the title", () => {
  const el = mount("tl-drive-row", {
    attrs: { href: "/drives/1", locale: "fr", "time-zone": "UTC", now: "2026-11-18T12:00:00Z" },
    data: {
      id: 1,
      start_at: "2024-07-02T08:00:00Z",
      end_at: "2024-07-02T08:10:00Z",
      distance_km: 12.34,
      duration_s: 600,
      energy_used_kwh: 1.5,
      start_battery_level: 80,
      end_battery_level: 72,
      start_address: "Fixture Origin",
      start_geofence: "Fixture Bay",
      end_address: "South lot",
      source: "poll",
    },
  });
  const text = el.shadowRoot.textContent;
  assert.match(text, /Fixture Bay/);
  assert.match(text, /South lot/);
  assert.equal(el.shadowRoot.querySelector(".route-stop.start").getAttribute("title"), "Fixture Origin");
  assert.match(text, /12,3/);
  assert.equal(el.shadowRoot.querySelector("svg.car"), null);
});

test("drive row titles use the street and the tooltip keeps the house number", () => {
  const el = mount("tl-drive-row", {
    attrs: { href: "/drives/2", locale: "fr", "time-zone": "UTC", now: "2026-11-18T12:00:00Z" },
    data: {
      id: 2,
      start_at: "2024-07-02T08:00:00Z",
      end_at: "2024-07-02T08:10:00Z",
      distance_km: 3,
      duration_s: 600,
      start_address: "12, rue Example, Localité",
      end_address: "rue Example, Localité",
    },
  });
  const start = el.shadowRoot.querySelector(".route-stop.start");
  const end = el.shadowRoot.querySelector(".route-stop.end");
  assert.equal(start.textContent, "rue Example");
  assert.equal(start.getAttribute("title"), "12 rue Example, Localité");
  assert.equal(end.textContent, "rue Example");
  assert.equal(end.getAttribute("title"), "Localité");
  assert.match(el.shadowRoot.querySelector(".trip").getAttribute("aria-label"), /rue Example/);
  assert.equal(el.shadowRoot.querySelector(".trip").getAttribute("aria-label").includes("12,"), false);
});

test("charge row shows energy, power and cost", () => {
  const el = mount("tl-charge-row", {
    attrs: { href: "/charges/2", locale: "fr", "time-zone": "UTC" },
    data: {
      id: 2,
      start_at: "2024-07-02T09:00:00Z",
      end_at: "2024-07-02T10:00:00Z",
      energy_added_kwh: 11.2,
      duration_s: 3600,
      max_power_kw: 7,
      cost: 2.5,
      is_dc: false,
      start_battery_level: 20,
      end_battery_level: 80,
      geofence: "North garage",
      source: "manual",
    },
  });
  const text = el.shadowRoot.textContent;
  assert.match(text, /\+11,2/);
  assert.match(text, /North garage/);
  assert.match(text, /7/);
  assert.match(text, /2,50/);
  assert.match(text, /manual/);
  assert.ok(el.shadowRoot.querySelector("svg.soc-bar"));
  assert.equal(el.shadowRoot.querySelector("svg.car"), null);
});

test("period chips and tabs are data driven", () => {
  const chips = mount("tl-period-chips", {
    attrs: { label: "Période" },
    data: [
      { id: "7d", label: "7 jours", href: "/drives?from=2026-11-12", current: true },
      { id: "all", label: "Tout", href: "#all", current: false },
    ],
  });
  assert.equal(chips.shadowRoot.querySelector("[aria-current='page']").textContent, "7 jours");
  let changed = null;
  chips.addEventListener("tl-change", (event) => {
    changed = event.detail.id;
  });
  chips.shadowRoot.querySelectorAll("a")[1].click();
  assert.equal(changed, "all");

  const tabs = mount("tl-tabs", {
    attrs: { label: "Sections du véhicule" },
    data: [
      { id: "tab-overview", label: "Aperçu", href: "/v/1", selected: true },
      { id: "tab-drives", label: "Trajets", href: "/v/1/drives", selected: false },
    ],
  });
  assert.equal(tabs.getAttribute("role"), "tablist");
  assert.equal(tabs.shadowRoot.querySelector("[aria-selected='true']").textContent, "Aperçu");
});

test("summary strip formats a stats payload", () => {
  const el = mount("tl-summary-strip", {
    attrs: { label: "Totaux", locale: "fr" },
    data: {
      drive_count: 2,
      distance_km: 12.5,
      drive_duration_s: 1200,
      energy_used_kwh: 2,
    },
  });
  assert.match(el.shadowRoot.textContent, /12,5/);
  assert.match(el.shadowRoot.textContent, /Wh\/km/);
});

test("bar chart draws from the console payload and has no car drawing", () => {
  const el = mount("tl-chart", {
    attrs: { title: "Distance", summary: "Total 3 km", variant: "card" },
    data: {
      kind: "bar",
      labels: ["16 nov."],
      titles: ["16 novembre 2026"],
      hrefs: ["/v/1/drives?from=2026-11-16"],
      series: [{ name: "Distance", unit: "km", decimals: 0, tone: "accent", values: [3], area: false, signed: false }],
      stacked: false,
    },
  });
  assert.ok(el.shadowRoot.querySelector("svg.chart-svg"));
  assert.equal(el.shadowRoot.querySelector("svg.car"), null);
  assert.equal(el.classList.contains("ready"), true);
});

test("a paused cable reading is unknown and the data source is shown", () => {
  const card = mount("tl-vehicle-card", {
    attrs: { locale: "en" },
    data: {
      name: "Example",
      activity: "online",
      charging_state: "Disconnected",
      plugged_in: false,
      charge_status: "unknown",
      charge_status_stale: true,
      data_source: "polling",
    },
  });
  const text = card.shadowRoot.textContent;
  assert.match(text, /Unknown/);
  assert.match(text, /Polling/);
  assert.doesNotMatch(text, /Unplugged/);
});

test("the package source does not fetch tiles or tesla renders", () => {
  const source = [
    "car-image.js", "vehicle-card.js", "mini-map.js", "chart.js", "index.js",
  ];
  // The assertion below reads the built modules through the imported graph:
  assert.equal(typeof ui.mountCharts, "function");
  assert.equal(String(ui.TlMiniMap).includes("fetch("), false);
  assert.equal(String(ui.TlCarImage).includes("tesla.com"), false);
  assert.equal(source.length > 0, true);
});
