// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

export const base = `
:host {
  color: var(--tl-text, #171a20);
  font-family: var(--tl-font, -apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif);
  font-size: 1rem;
  line-height: 1.5;
  box-sizing: border-box;
}
:host([theme="light"]) {
  color-scheme: light;
  --tl-bg: #f4f4f4; --tl-surface: #fff; --tl-surface-2: #f4f4f4; --tl-surface-3: #e9e9eb;
  --tl-line: rgba(23, 26, 32, 0.08); --tl-line-strong: rgba(23, 26, 32, 0.16);
  --tl-text: #171a20; --tl-muted: #5c5e62; --tl-faint: #8e9095;
  --tl-accent: #3e6ae1; --tl-accent-strong: #2f56c6; --tl-accent-soft: rgba(62, 106, 225, 0.1); --tl-on-accent: #fff;
  --tl-green: #1d9b52; --tl-green-soft: rgba(29, 155, 82, 0.12);
  --tl-amber: #c27c0e; --tl-violet: #7258e8; --tl-slate: #6b7280;
  --tl-danger: #c8332f; --tl-danger-soft: rgba(200, 51, 47, 0.1);
  --tl-map-bg: #eef0f3; --tl-map-dot: rgba(23, 26, 32, 0.12); --tl-map-road: rgba(23, 26, 32, 0.16);
  --tl-shadow: 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04);
}
:host([theme="dark"]) {
  color-scheme: dark;
  --tl-bg: #111318; --tl-surface: #181b21; --tl-surface-2: #20242b; --tl-surface-3: #2a2e36;
  --tl-line: rgba(255, 255, 255, 0.07); --tl-line-strong: rgba(255, 255, 255, 0.14);
  --tl-text: #f2f3f5; --tl-muted: #a3a6ad; --tl-faint: #7b7f87;
  --tl-accent: #6f93ff; --tl-accent-strong: #8fabff; --tl-accent-soft: rgba(111, 147, 255, 0.14); --tl-on-accent: #0b1430;
  --tl-green: #4fd17c; --tl-green-soft: rgba(79, 209, 124, 0.14);
  --tl-amber: #f2b441; --tl-violet: #a892ff; --tl-slate: #9aa1ad;
  --tl-danger: #ff7a70; --tl-danger-soft: rgba(255, 122, 112, 0.12);
  --tl-map-bg: #1d2128; --tl-map-dot: rgba(255, 255, 255, 0.08); --tl-map-road: rgba(255, 255, 255, 0.14);
  --tl-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
}
:host *, :host *::before, :host *::after { box-sizing: border-box; }
:focus { outline: none; }
:focus-visible { outline: 2px solid var(--tl-accent, #3e6ae1); outline-offset: 2px; border-radius: 6px; }
.visually-hidden { position: absolute !important; clip: rect(0 0 0 0); clip-path: inset(50%); height: 1px; overflow: hidden; white-space: nowrap; width: 1px; }
.muted { color: var(--tl-muted, #5c5e62); }
.figure, time { font-variant-numeric: tabular-nums; }
h2, h3 { font-weight: 500; letter-spacing: -0.01em; line-height: 1.2; margin: 0; }
h2 { font-size: 1.125rem; }
a { color: var(--tl-accent, #3e6ae1); }
.card-head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 8px 16px; margin-bottom: 12px; }
.card-head h2 { margin: 0; }
.text-link { color: var(--tl-accent, #3e6ae1); font-size: 0.875rem; font-weight: 500; text-decoration: none; }
.text-link:hover { text-decoration: underline; }
@keyframes tl-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
@keyframes tl-ripple { 0% { transform: scale(0.5); opacity: 0.6; } 100% { transform: scale(1.6); opacity: 0; } }
@keyframes tl-draw { from { stroke-dashoffset: 1; } to { stroke-dashoffset: 0; } }
@media (prefers-reduced-motion: reduce) {
  :host, :host *, :host *::before, :host *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
`;

export const card = `
:host {
  display: block;
  position: relative;
  width: 100%;
  min-width: 0;
  margin: 0;
  padding: 24px;
  border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08));
  border-radius: var(--tl-radius, 14px);
  background: var(--tl-surface, #fff);
  box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04));
}
.eyebrow { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin: 0; color: var(--tl-muted, #5c5e62); font-size: 0.75rem; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; }
.hero-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
.hero-name { margin: 6px 0 0; font-size: clamp(1.5rem, 5vw, 2rem); font-weight: 500; letter-spacing: -0.02em; }
.hero-car { margin: 16px -12px 8px; }
.hero-energy { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; margin: 0 0 12px; }
.hero-energy p { display: flex; flex-direction: column; margin: 0; }
.hero-range { text-align: right; align-items: flex-end; }
.hero-figure { font-size: clamp(2rem, 7vw, 2.75rem); font-weight: 400; letter-spacing: -0.03em; line-height: 1.05; font-variant-numeric: tabular-nums; }
.hero-label { color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.hero-note { margin: 12px 0 0; color: var(--tl-muted, #5c5e62); font-size: 0.9375rem; }
dl.hero-facts { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px 16px; margin: 20px 0 0; padding-top: 16px; border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
dt { color: var(--tl-muted, #5c5e62); font-size: 0.75rem; font-weight: 500; }
dd { margin: 2px 0 0; overflow-wrap: anywhere; }
.chip { display: inline-flex; align-items: center; gap: 6px; min-height: 28px; padding: 0 12px; border-radius: 999px; background: var(--tl-surface-2, #f4f4f4); color: var(--tl-text, #171a20); font-size: 0.8125rem; font-weight: 500; line-height: 1.2; white-space: nowrap; }
.status-dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; flex: none; }
.status.drive { background: var(--tl-accent-soft, rgba(62, 106, 225, 0.1)); color: var(--tl-accent, #3e6ae1); }
.status.charge { background: var(--tl-green-soft, rgba(29, 155, 82, 0.12)); color: var(--tl-green, #1d9b52); }
.status.park { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-text, #171a20); }
.status.note { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-amber, #c27c0e); }
.status.idle { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-muted, #5c5e62); }
.status.drive .status-dot, .status.charge .status-dot { animation: tl-pulse 2s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
`;

export const photo = `
:host { display: block; width: 100%; }
.stage { display: grid; aspect-ratio: 2 / 1; overflow: hidden; border-radius: var(--tl-radius-sm, 10px); background: var(--tl-surface-2, #f4f4f4); }
.stage > * { grid-area: 1 / 1; }
img { display: none; width: 100%; height: 100%; object-fit: cover; scale: 1.14;
  -webkit-mask-image: radial-gradient(ellipse 47% 60% at 50% 54%, #000 62%, transparent 100%);
  mask-image: radial-gradient(ellipse 47% 60% at 50% 54%, #000 62%, transparent 100%); }
:host(.ready) img { display: block; }
:host(.ready) .placeholder { display: none; }
.placeholder { display: flex; align-items: center; justify-content: center; margin: 0; padding: 16px; color: var(--tl-faint, #8e9095); font-size: 0.875rem; line-height: 1.4; text-align: center; }
`;

export const battery = `
:host { display: block; width: 100%; }
svg.battery-meter { display: block; width: 100%; height: 8px; overflow: visible; }
.battery-track { fill: var(--tl-surface-3, #e9e9eb); }
.battery-fill { fill: var(--tl-text, #171a20); }
.battery-meter.charging .battery-fill { fill: var(--tl-green, #1d9b52); }
.battery-meter.low .battery-fill { fill: var(--tl-danger, #c8332f); }
.battery-limit { fill: var(--tl-muted, #5c5e62); }
`;

export const map = `
:host { display: block; width: 100%; }
.map-stack { position: relative; overflow: hidden; background: var(--tl-map-bg, #eef0f3); }
.tiles { position: absolute; inset: 0; }
.tiles ::slotted(*) { width: 100%; height: 100%; object-fit: cover; }
svg.mini-map { position: relative; display: block; width: 100%; height: auto; aspect-ratio: 360 / 220; }
.map-bg { fill: var(--tl-map-bg, #eef0f3); }
.map-dot { fill: var(--tl-map-dot, rgba(23, 26, 32, 0.12)); }
.map-vignette { fill: var(--tl-map-bg, #eef0f3); opacity: 0.9; }
.map-road { fill: none; stroke: var(--tl-map-road, rgba(23, 26, 32, 0.16)); stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; }
.map-halo { fill: var(--tl-text, #171a20); opacity: 0.1; }
.mini-map.live .map-halo { fill: var(--tl-accent, #3e6ae1); opacity: 0.18; }
.mini-map.empty .map-halo { fill: var(--tl-faint, #8e9095); }
.map-heading { fill: var(--tl-text, #171a20); opacity: 0.75; }
.mini-map.live .map-heading { fill: var(--tl-accent, #3e6ae1); opacity: 1; }
.map-car { fill: var(--tl-text, #171a20); stroke: var(--tl-surface, #fff); stroke-width: 3; }
.mini-map.live .map-car { fill: var(--tl-accent, #3e6ae1); }
.map-pulse { fill: none; stroke: var(--tl-text, #171a20); stroke-width: 1.5; opacity: 0; transform-box: fill-box; transform-origin: center; }
.mini-map.live .map-pulse { stroke: var(--tl-accent, #3e6ae1); animation: tl-ripple 2.4s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
.map-scale path { fill: none; stroke: var(--tl-muted, #5c5e62); stroke-width: 1.5; }
.map-scale text { fill: var(--tl-muted, #5c5e62); font-family: var(--tl-font, sans-serif); font-size: 11px; dominant-baseline: middle; }
:host(.has-tiles) .map-bg, :host(.has-tiles) .map-dots { display: none; }
`;

export const trip = `
:host { display: block; background: transparent; }
:host(:not(:first-child)) { border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
a.trip, .trip {
  display: grid;
  grid-template-columns: 3.25rem minmax(0, 1fr) auto;
  grid-template-areas: "times route figures" "times stats stats";
  gap: 6px 14px; align-items: start; width: 100%; padding: 14px 16px;
  color: inherit; text-decoration: none; box-sizing: border-box;
}
a.trip:hover, .trip:hover { background: var(--tl-surface-2, #f4f4f4); }
a.trip:focus-visible { outline-offset: -2px; border-radius: 0; }
.trip-times { grid-area: times; display: flex; flex-direction: column; gap: 2px; font-size: 0.875rem; }
.trip-times time { font-weight: 600; }
.trip-times > span { color: var(--tl-faint, #8e9095); }
.live { display: inline-flex; align-items: center; gap: 6px; color: var(--tl-green, #1d9b52); font-weight: 500; font-size: 0.75rem; }
.live-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--tl-green, #1d9b52); animation: tl-pulse 2s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
.route { grid-area: route; position: relative; display: flex; flex-direction: column; gap: 4px; min-width: 0; }
.route::before { content: ""; position: absolute; left: 3.25px; top: 0.65em; bottom: 0.65em; width: 1.5px; background: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); }
.route.single::before { display: none; }
.route-stop { position: relative; display: block; padding-left: 18px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.route.single .route-stop { padding-left: 0; }
.route-stop::before { content: ""; position: absolute; left: 0; top: 0.5em; width: 8px; height: 8px; border-radius: 50%; box-sizing: border-box; }
.route-stop.start::before { border: 2px solid var(--tl-muted, #5c5e62); background: var(--tl-surface, #fff); }
.route-stop.end::before { background: var(--tl-accent, #3e6ae1); }
.route .charge-stop::before { display: none; }
.kind { display: flex; align-items: center; gap: 4px; font-size: 0.8125rem; color: var(--tl-muted, #5c5e62); }
.trip-figures { grid-area: figures; display: flex; flex-direction: column; align-items: flex-end; gap: 2px; text-align: right; font-size: 0.875rem; }
.trip-figures .figure { font-size: 1.0625rem; font-weight: 600; color: var(--tl-text, #171a20); white-space: nowrap; }
.trip.charge .trip-figures .figure { color: var(--tl-green, #1d9b52); }
.trip-stats { grid-area: stats; display: flex; flex-wrap: wrap; gap: 4px 18px; color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.trip-stats > span { display: flex; flex-direction: column; }
.stat-label { display: none; }
.chip.source { grid-column: 2 / -1; justify-self: start; min-height: 22px; font-size: 0.75rem; color: var(--tl-muted, #5c5e62); }
.chip.dc { min-height: 20px; padding: 0 7px; background: var(--tl-green-soft, rgba(29, 155, 82, 0.12)); color: var(--tl-green, #1d9b52); font-size: 0.6875rem; font-weight: 700; letter-spacing: 0.04em; }
svg.soc-bar { display: block; width: 100%; max-width: 9rem; height: 6px; margin-top: 6px; }
.soc-track { fill: var(--tl-surface-3, #e9e9eb); }
.soc-fill { fill: var(--tl-text, #171a20); }
.soc-bar.gain .soc-fill { fill: var(--tl-green, #1d9b52); }
@media (max-width: 640px) {
  a.trip, .trip { grid-template-columns: 2.75rem minmax(0, 1fr) auto; padding: 12px 14px; gap: 4px 12px; }
}
@media (min-width: 900px) {
  a.trip, .trip {
    grid-template-columns: 3.5rem minmax(0, 1.4fr) minmax(0, 1.6fr) 6.5rem;
    grid-template-areas: "times route stats figures";
    align-items: center;
  }
  .trip-stats { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; color: var(--tl-text, #171a20); font-size: 0.875rem; }
  .stat-label { display: block; color: var(--tl-faint, #8e9095); font-size: 0.6875rem; letter-spacing: 0.02em; }
  .chip.source { grid-column: 2; grid-row: 2; }
}
`;

export const day = `
:host { display: block; margin: 0 0 24px; }
.day-head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 4px 12px; padding: 0 4px 8px; }
.day-head h2 { margin: 0; font-size: 1rem; font-weight: 600; }
.day-head p { margin: 0; font-size: 0.8125rem; }
.trips { overflow: hidden; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
`;

export const recent = `
:host { display: block; padding: 24px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
.recent-link { display: block; margin: 0 -12px -12px; padding: 12px; border-radius: 10px; color: inherit; text-decoration: none; }
.recent-link:hover { background: var(--tl-surface-2, #f4f4f4); }
.recent-when { margin: 0 0 10px; color: var(--tl-muted, #5c5e62); font-size: 0.875rem; }
.recent-metrics { display: flex; align-items: baseline; gap: 12px; margin: 12px 0 0; }
.recent-metrics .figure { font-size: 1.375rem; font-weight: 500; letter-spacing: -0.02em; }
.route { position: relative; display: flex; flex-direction: column; gap: 4px; min-width: 0; margin-left: 2px; }
.route::before { content: ""; position: absolute; left: 3.25px; top: 0.65em; bottom: 0.65em; width: 1.5px; background: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); }
.route.single::before { display: none; }
.route-stop { position: relative; display: block; padding-left: 18px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.route.single .route-stop { padding-left: 0; }
.route-stop::before { content: ""; position: absolute; left: 0; top: 0.5em; width: 8px; height: 8px; border-radius: 50%; }
.route-stop.start::before { border: 2px solid var(--tl-muted, #5c5e62); background: var(--tl-surface, #fff); }
.route-stop.end::before { background: var(--tl-accent, #3e6ae1); }
.live-dot { display: inline-block; width: 7px; height: 7px; border-radius: 50%; background: var(--tl-green, #1d9b52); }
`;

export const summary = `
:host { display: block; }
:host([variant="strip"]) { margin: 0 0 24px; }
:host([variant="card"]) { padding: 24px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
dl { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap: 24px; margin: 0; }
dl > div { padding: 12px 0; border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
dd { margin-top: 4px; font-size: 1.125rem; font-weight: 500; letter-spacing: -0.01em; font-variant-numeric: tabular-nums; }
:host([variant="strip"]) dl { padding: 4px 20px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
:host([variant="strip"]) dl > div { border-top: 0; }
@media (min-width: 720px) {
  :host([variant="strip"]) dl { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}
@media (min-width: 900px) {
  :host([variant="card"]) dl { grid-template-columns: repeat(4, minmax(0, 1fr)); }
}
`;

export const chips = `
:host { display: flex; flex-wrap: nowrap; gap: 2px; overflow-x: auto; scrollbar-width: none; padding: 3px; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); width: max-content; max-width: 100%; }
:host::-webkit-scrollbar { display: none; }
a { display: inline-flex; align-items: center; min-height: 32px; padding: 0 14px; border-radius: 999px; color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; font-weight: 500; text-decoration: none; white-space: nowrap; }
a:hover { color: var(--tl-text, #171a20); }
a[aria-current="page"] { background: var(--tl-surface, #fff); color: var(--tl-text, #171a20); box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12); }
`;

export const tabs = `
:host { display: flex; flex-wrap: nowrap; gap: 4px; overflow-x: auto; scrollbar-width: none; margin: 0 0 24px; padding: 4px; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); width: max-content; max-width: 100%; }
:host::-webkit-scrollbar { display: none; }
a { display: inline-flex; align-items: center; justify-content: center; min-height: 36px; padding: 0 16px; border-radius: 999px; color: var(--tl-muted, #5c5e62); font-size: 0.875rem; font-weight: 500; text-decoration: none; white-space: nowrap; }
a:hover { color: var(--tl-text, #171a20); }
a[aria-selected="true"] { background: var(--tl-surface, #fff); color: var(--tl-text, #171a20); box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12); }
`;

export const chart = `
:host { display: block; position: relative; color: var(--tl-text, #171a20); }
:host([variant="card"]) {
  padding: 20px;
  border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08));
  border-radius: var(--tl-radius, 14px);
  background: var(--tl-surface, #fff);
  box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04));
}
figure.chart { position: relative; margin: 0; padding: 0; }
figure.chart figcaption { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 2px 12px; margin: 0 0 8px; }
.chart-title { font-size: 0.9375rem; font-weight: 600; }
.chart-summary { color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.chart-plot { position: relative; min-height: 180px; touch-action: pan-y; user-select: none; }
svg.chart-svg { display: block; width: 100%; overflow: visible; }
.chart-grid-line { stroke: var(--tl-line, rgba(23, 26, 32, 0.08)); stroke-width: 1; shape-rendering: crispEdges; }
.chart-zero { stroke: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); stroke-width: 1; shape-rendering: crispEdges; }
.chart-axis text, .chart-tick { fill: var(--tl-faint, #8e9095); font-family: var(--tl-font, sans-serif); font-size: 11px; font-variant-numeric: tabular-nums; }
.chart-line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; vector-effect: non-scaling-stroke; stroke: currentColor; }
.chart-area { stroke: none; }
.chart-bar { fill: currentColor; transition: opacity 160ms cubic-bezier(0.2, 0.7, 0.2, 1); }
.chart-bars:hover .chart-bar { opacity: 0.45; }
.chart-bars .chart-bar.on, .chart-bars:hover .chart-bar.on { opacity: 1; }
.chart-bar-hit { fill: transparent; cursor: pointer; }
.chart-crosshair { stroke: var(--tl-muted, #5c5e62); stroke-width: 1; stroke-dasharray: 3 3; opacity: 0; shape-rendering: crispEdges; }
.chart-dot { fill: currentColor; stroke: var(--tl-surface, #fff); stroke-width: 2; opacity: 0; }
.chart-on .chart-crosshair, .chart-on .chart-dot { opacity: 1; }
.chart-brush { fill: var(--tl-accent, #3e6ae1); opacity: 0.12; }
.tone-accent { color: var(--tl-accent, #3e6ae1); }
.tone-green { color: var(--tl-green, #1d9b52); }
.tone-amber { color: var(--tl-amber, #c27c0e); }
.tone-violet { color: var(--tl-violet, #7258e8); }
.tone-slate { color: var(--tl-slate, #6b7280); }
.chart-tooltip { position: absolute; top: 0; left: 0; z-index: 3; min-width: 8rem; padding: 8px 10px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: 10px; background: var(--tl-surface, #fff); box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12); color: var(--tl-text, #171a20); font-size: 0.8125rem; line-height: 1.35; pointer-events: none; opacity: 0; white-space: nowrap; }
.chart-on .chart-tooltip { opacity: 1; }
.chart-tooltip .tip-head { margin-bottom: 4px; color: var(--tl-muted, #5c5e62); }
.chart-tooltip .tip-row { display: flex; align-items: center; gap: 6px; }
.chart-tooltip .tip-row b { margin-left: auto; padding-left: 12px; font-weight: 600; }
.chart-tooltip .tip-swatch, .chart-legend .tip-swatch { width: 8px; height: 8px; border-radius: 2px; background: currentColor; }
.chart-legend { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: -2px 0 8px; color: var(--tl-muted, #5c5e62); font-size: 0.75rem; }
.chart-legend > span { display: inline-flex; align-items: center; gap: 6px; }
.chart-tools { position: absolute; top: 8px; right: 0; display: flex; gap: 4px; opacity: 0; }
figure.chart.zoomed .chart-tools { opacity: 1; }
figure.chart:not(.chart-lead) .chart-tools { display: none; }
.chart-tools button { min-height: 28px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); color: var(--tl-text, #171a20); font: inherit; font-size: 0.75rem; cursor: pointer; }
.chart-ready .chart-fallback { display: none; }
svg.chart-fallback { display: block; width: 100%; height: 180px; color: var(--tl-accent, #3e6ae1); }
`;

export function sheet(...parts) {
  return base + parts.join("\n");
}
