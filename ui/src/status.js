// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

function clockMinute(minutes) {
  if (minutes == null || !Number.isFinite(Number(minutes))) return "";
  const value = Math.trunc(Number(minutes));
  if (value < 0 || value > 1440) return "";
  const normalized = value === 1440 ? 0 : value;
  const hour = Math.floor(normalized / 60);
  const minute = normalized % 60;
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}

function fill(template, time) {
  return String(template || "").replace("{time}", time);
}

/** Local-clock schedule. Empty when charging is off and off-peak is not enabled. */
export function scheduleFrom(data, strings) {
  if (!data) return "";
  if (data.schedule) return String(data.schedule);
  const mode = String(data.scheduled_charging_mode || "").trim().toLowerCase();
  const off = mode === "off" || mode === "scheduledchargingmodeoff";
  const depart = mode === "departby" || mode === "depart_by" || mode === "scheduledchargingmodedepartby";
  const startAt = mode === "startat" || mode === "start_at" || mode === "scheduledchargingmodestartat";
  const parts = [];
  if (!off) {
    if (depart) {
      const time = clockMinute(data.scheduled_departure_time);
      parts.push(time ? fill(strings.departAt, time) : strings.departScheduled);
    } else if (startAt || (mode === "" && data.scheduled_charging_start_time != null)) {
      const time = clockMinute(data.scheduled_charging_start_time);
      if (time) parts.push(fill(strings.chargeAt, time));
      else if (startAt) parts.push(strings.chargeScheduled);
    }
  }
  if (data.off_peak_charging_enabled === true) {
    const time = clockMinute(data.off_peak_hours_end_time);
    parts.push(time ? fill(strings.offPeakUntil, time) : strings.offPeak);
  }
  return parts.join(" · ");
}

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
  const pluggedIdle = charging === "starting" || charging === "complete" || charging === "stopped" || charging === "nopower";
  let key = "unknown";
  if (activity === "driving") key = "driving";
  else if (charging === "charging") key = "charging";
  else if (activity === "updating") key = "updating";
  else if (activity === "asleep") key = "asleep";
  else if (activity === "offline") key = "offline";
  else if (pluggedIdle) key = "plugged";
  else if (activity === "charging" && charging !== "disconnected") key = "charging";
  else if (charging !== "disconnected" && data?.plugged_in === true) key = "plugged";
  else if (activity === "online" || activity === "parked" || activity === "charging") key = "parked";
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
