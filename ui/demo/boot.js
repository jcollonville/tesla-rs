import "../dist/tesla-logger-ui.js";

const overview = document.querySelector("#overview");

const card = document.createElement("tl-vehicle-card");
card.locale = "fr";
card.timeZone = "Europe/Paris";
card.unavailable = "Image du véhicule indisponible";
card.data = {
  name: "Example",
  model: "Model Y",
  trim: "Long Range",
  activity: "parked",
  battery_level: 72,
  charge_limit_soc: 80,
  rated_range_km: 310,
  note: "Limite de charge 80 %",
  locked: true,
  inside_temp_c: 21.5,
  outside_temp_c: 8,
  odometer_km: 18420,
};

const missing = document.createElement("tl-vehicle-card");
missing.locale = "fr";
missing.imageUrl = "/does-not-exist.png";
missing.data = {
  name: "Sans image",
  model: "Model 3",
  activity: "asleep",
  battery_level: 41,
  battery_text: "41 %",
  range_text: "180 km",
  facts: [{ label: "Logiciel", value: "2026.1" }],
};

const place = document.createElement("section");
place.className = "place";
const map = document.createElement("tl-mini-map");
map.data = {
  label: "Position : North garage",
  empty: false,
  moving: false,
  width: 360,
  height: 220,
  roads: ["M30 150 C80 120 140 90 190 110 L280 140"],
  heading: 40,
  scale_width: 48,
  scale_label: "200 m",
};
const caption = document.createElement("p");
caption.textContent = "North garage";
place.append(map, caption);

const recent = document.createElement("tl-recent-card");
recent.locale = "fr";
recent.timeZone = "Europe/Paris";
recent.setAttribute("now", "2026-11-18T12:00:00Z");
recent.title = "Dernier trajet";
recent.setAttribute("link-href", "#drives");
recent.setAttribute("link-label", "Tous les trajets");
recent.data = {
  start_at: "2026-11-18T07:12:00Z",
  end_at: "2026-11-18T07:40:00Z",
  distance_km: 18.4,
  duration_s: 1680,
  energy_used_kwh: 3.1,
  start_geofence: "North garage",
  end_geofence: "South lot",
  href: "#drive",
};

const month = document.createElement("tl-summary-strip");
month.locale = "fr";
month.setAttribute("variant", "card");
month.setAttribute("title", "Ce mois-ci");
month.setAttribute("label", "Ce mois-ci");
month.data = {
  drive_count: 12,
  distance_km: 246,
  drive_duration_s: 28000,
  energy_used_kwh: 42,
};

overview.append(card, place, recent, missing, month);

const tabs = document.createElement("tl-tabs");
tabs.setAttribute("label", "Sections du véhicule");
tabs.data = [
  { id: "tab-overview", label: "Aperçu", href: "#overview", selected: true },
  { id: "tab-drives", label: "Trajets", href: "#drives", selected: false },
  { id: "tab-charges", label: "Charges", href: "#charges", selected: false },
  { id: "tab-stats", label: "Stats", href: "#charts", selected: false },
];
document.querySelector("#tabs").append(tabs);

const chips = document.createElement("tl-period-chips");
chips.setAttribute("label", "Période");
chips.data = [
  { id: "7d", label: "7 jours", href: "#7", current: false },
  { id: "30d", label: "30 jours", href: "#30", current: true },
  { id: "year", label: "Cette année", href: "#year", current: false },
  { id: "all", label: "Tout", href: "#all", current: false },
];
document.querySelector("#periods").append(chips);

const totals = document.createElement("tl-summary-strip");
totals.locale = "fr";
totals.setAttribute("variant", "strip");
totals.setAttribute("label", "Totaux de la sélection");
totals.data = [
  { label: "Trajets", value: "2" },
  { label: "Distance", value: "30,4\u202fkm" },
  { label: "Au volant", value: "48\u202fmin" },
  { label: "Énergie", value: "5,2\u202fkWh" },
  { label: "Efficacité", value: "171\u202fWh/km" },
];
document.querySelector("#totals").append(totals);

const day = document.createElement("tl-day-group");
day.setAttribute("heading", "Aujourd'hui");
day.setAttribute("summary", "2 trajets · 30,4\u202fkm · 48\u202fmin");
for (const row of [
  {
    id: 1,
    start_at: "2026-11-18T07:12:00Z",
    end_at: "2026-11-18T07:40:00Z",
    distance_km: 18.4,
    duration_s: 1680,
    energy_used_kwh: 3.1,
    start_battery_level: 80,
    end_battery_level: 71,
    start_geofence: "North garage",
    start_address: "Fixture Origin",
    end_geofence: "South lot",
    source: "poll",
  },
  {
    id: 2,
    start_at: "2026-11-18T17:05:00Z",
    end_at: "2026-11-18T17:25:00Z",
    distance_km: 12,
    duration_s: 1200,
    energy_used_kwh: 2.1,
    start_battery_level: 70,
    end_battery_level: 64,
    start_geofence: "South lot",
    end_geofence: "North garage",
    source: "poll",
  },
]) {
  const el = document.createElement("tl-drive-row");
  el.locale = "fr";
  el.timeZone = "UTC";
  el.setAttribute("now", "2026-11-18T18:00:00Z");
  el.href = `#drive-${row.id}`;
  el.data = row;
  day.append(el);
}
document.querySelector("#days").append(day);

const chargeDay = document.createElement("tl-day-group");
chargeDay.setAttribute("heading", "Hier");
chargeDay.setAttribute("summary", "1 charge · +11,2\u202fkWh");
const charge = document.createElement("tl-charge-row");
charge.locale = "fr";
charge.timeZone = "UTC";
charge.href = "#charge-1";
charge.data = {
  id: 3,
  start_at: "2026-11-17T21:00:00Z",
  end_at: "2026-11-17T23:10:00Z",
  energy_added_kwh: 11.2,
  duration_s: 7800,
  max_power_kw: 7,
  cost: 2.5,
  is_dc: false,
  charger_type: "AC",
  start_battery_level: 28,
  end_battery_level: 80,
  geofence: "North garage",
  source: "poll",
};
chargeDay.append(charge);
document.querySelector("#charges").append(chargeDay);

const chart = document.createElement("tl-chart");
chart.setAttribute("variant", "card");
chart.setAttribute("title", "Distance");
chart.setAttribute("summary", "Total 30 km · max 18 km");
chart.data = {
  kind: "bar",
  labels: ["16 nov.", "17 nov.", "18 nov."],
  titles: ["16 novembre", "17 novembre", "18 novembre"],
  hrefs: ["#a", "#b", "#c"],
  series: [{ name: "Distance", unit: "km", decimals: 0, tone: "accent", values: [12, 0, 18] }],
};
document.querySelector("#charts").append(chart);
