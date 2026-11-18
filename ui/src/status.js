// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

export function statusFrom(data, strings) {
  if (data && data.status_label) {
    return {
      label: data.status_label,
      tone: data.status_tone || "idle",
      charging: Boolean(data.charging),
    };
  }
  const activity = String(data?.activity || "").toLowerCase();
  const charging = String(data?.charging_state || "").toLowerCase();
  let key = "unknown";
  if (activity === "driving") key = "driving";
  else if (activity === "charging" || charging === "charging" || charging === "starting") key = "charging";
  else if (activity === "updating") key = "updating";
  else if (activity === "asleep") key = "asleep";
  else if (activity === "offline") key = "offline";
  else if (data?.plugged_in === true || charging === "complete" || charging === "stopped" || charging === "nopower") key = "plugged";
  else if (activity === "online" || activity === "parked") key = "parked";
  const table = {
    driving: ["drive", strings.driving],
    charging: ["charge", strings.charging],
    plugged: ["charge", strings.plugged],
    parked: ["park", strings.parked],
    updating: ["note", strings.updating],
    asleep: ["idle", strings.asleep],
    offline: ["idle", strings.offline],
    unknown: ["idle", strings.unknown],
  };
  const [tone, label] = table[key];
  return { label, tone, charging: key === "charging" };
}
