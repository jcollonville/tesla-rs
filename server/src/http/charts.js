// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

import { mountCharts } from "/static/tesla-logger-ui.js";

function boot() {
  mountCharts(document);
}

if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
else boot();
