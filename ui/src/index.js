// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlBattery } from "./battery.js";
import { TlCarImage } from "./car-image.js";
import { TlChart } from "./chart.js";
import { mountChart, mountCharts } from "./chart-engine.js";
import { TlPeriodChips, TlSummaryStrip, TlTabs } from "./chrome.js";
import { TlMiniMap } from "./mini-map.js";
import { TlChargeRow, TlDayGroup, TlDriveRow, TlRecentCard } from "./rows.js";
import { TlVehicleCard } from "./vehicle-card.js";

const TAGS = [
  ["tl-battery", TlBattery],
  ["tl-car-image", TlCarImage],
  ["tl-vehicle-card", TlVehicleCard],
  ["tl-mini-map", TlMiniMap],
  ["tl-drive-row", TlDriveRow],
  ["tl-charge-row", TlChargeRow],
  ["tl-day-group", TlDayGroup],
  ["tl-recent-card", TlRecentCard],
  ["tl-summary-strip", TlSummaryStrip],
  ["tl-period-chips", TlPeriodChips],
  ["tl-tabs", TlTabs],
  ["tl-chart", TlChart],
];

export function defineElements(registry = customElements) {
  for (const [name, ctor] of TAGS) {
    if (!registry.get(name)) registry.define(name, ctor);
  }
}

defineElements();

export {
  TlBattery,
  TlCarImage,
  TlChart,
  TlChargeRow,
  TlDayGroup,
  TlDriveRow,
  TlMiniMap,
  TlPeriodChips,
  TlRecentCard,
  TlSummaryStrip,
  TlTabs,
  TlVehicleCard,
  mountChart,
  mountCharts,
};
export { makeFormat, NNBSP, number, duration, dayLabel } from "./format.js";
export { stringsFor } from "./i18n.js";
