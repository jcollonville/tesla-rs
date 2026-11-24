// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { stringsFor } from "./i18n.js";

export const NNBSP = "\u202f";

export function number(value, decimals) {
  if (!Number.isFinite(value)) return "—";
  const text = Math.abs(value).toFixed(decimals);
  const [whole, fraction] = text.split(".");
  let grouped = "";
  for (let index = 0; index < whole.length; index += 1) {
    if (index > 0 && (whole.length - index) % 3 === 0) grouped += NNBSP;
    grouped += whole[index];
  }
  const negative = value < 0 && [...text].some((ch) => ch >= "1" && ch <= "9");
  const sign = negative ? "−" : "";
  if (!fraction) return sign + grouped;
  return `${sign}${grouped},${fraction}`;
}

function intlNumber(locale, value, decimals) {
  if (!Number.isFinite(value)) return "—";
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(value);
}

export function duration(seconds, strings, sep = NNBSP) {
  if (seconds == null || !Number.isFinite(Number(seconds))) return "—";
  const total = Math.max(0, Math.trunc(Number(seconds)));
  if (total < 60) return `${total}${sep}${strings.second}`;
  const minutes = Math.trunc((total + 30) / 60);
  if (minutes < 60) return `${minutes}${sep}${strings.minute}`;
  const hours = Math.trunc(minutes / 60);
  const rest = minutes % 60;
  if (hours >= 24) {
    return `${Math.trunc(hours / 24)}${sep}${strings.day} ${hours % 24}${sep}${strings.hour}`;
  }
  return `${hours}${sep}${strings.hour}${sep}${String(rest).padStart(2, "0")}`;
}

export function efficiency(energyKwh, distanceKm) {
  if (energyKwh > 0 && distanceKm > 0.05) return (energyKwh * 1000) / distanceKm;
  return null;
}

export function clock(iso, timeZone) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: timeZone || "UTC",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(date);
  let hour = parts.find((part) => part.type === "hour")?.value || "00";
  const minute = parts.find((part) => part.type === "minute")?.value || "00";
  if (hour === "24") hour = "00";
  return `${hour.padStart(2, "0")}:${minute.padStart(2, "0")}`;
}

export function localDate(iso, timeZone) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: timeZone || "UTC",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  return {
    y: Number(parts.find((part) => part.type === "year").value),
    m: Number(parts.find((part) => part.type === "month").value),
    d: Number(parts.find((part) => part.type === "day").value),
  };
}

function sameDay(left, right) {
  return left && right && left.y === right.y && left.m === right.m && left.d === right.d;
}

function shiftDay(date, days) {
  const utc = new Date(Date.UTC(date.y, date.m - 1, date.d));
  utc.setUTCDate(utc.getUTCDate() + days);
  return { y: utc.getUTCFullYear(), m: utc.getUTCMonth() + 1, d: utc.getUTCDate() };
}

function weekdayIndex(date) {
  const utc = new Date(Date.UTC(date.y, date.m - 1, date.d));
  return (utc.getUTCDay() + 6) % 7;
}

function capitalize(text, locale) {
  if (!text) return "";
  const [first, ...rest] = [...text];
  return first.toLocaleUpperCase(locale) + rest.join("");
}

export function longDate(date, withYear, strings) {
  const day = strings.days[weekdayIndex(date)];
  const month = strings.months[date.m - 1];
  if (withYear) return `${day} ${date.d} ${month} ${date.y}`;
  return `${day} ${date.d} ${month}`;
}

export function dayLabel(iso, timeZone, nowIso, strings, locale) {
  const date = localDate(iso, timeZone);
  const today = localDate(nowIso || new Date().toISOString(), timeZone);
  if (!date || !today) return "—";
  if (sameDay(date, today)) return strings.today;
  if (sameDay(date, shiftDay(today, -1))) return strings.yesterday;
  return capitalize(longDate(date, date.y !== today.y, strings), locale);
}

export function ago(iso, nowIso, strings, sep = NNBSP) {
  if (!iso) return strings.never;
  const at = new Date(iso).getTime();
  const now = new Date(nowIso || Date.now()).getTime();
  if (!Number.isFinite(at) || !Number.isFinite(now)) return strings.never;
  const secs = Math.trunc((now - at) / 1000);
  if (secs < 45) return strings.justNow;
  const minutes = Math.trunc((secs + 30) / 60);
  if (minutes < 60) return `${strings.agoPrefix} ${minutes}${sep}${strings.minute}`;
  const hours = Math.trunc(minutes / 60);
  if (hours < 48) return `${strings.agoPrefix} ${hours}${sep}${strings.hour}`;
  return `${strings.agoPrefix} ${Math.trunc(hours / 24)}${sep}${strings.day}`;
}

const HOUSE_SUFFIXES = ["quater", "bis", "ter"];

function isDigit(ch) {
  return ch >= "0" && ch <= "9";
}

function isLetter(ch) {
  return (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z");
}

function boundary(text, index) {
  const next = text[index];
  return next == null || next === " " || next === "-" || next === "/";
}

// End index of one house-number token (`12`, `12 bis`, `12a`), or -1.
function eatHouseToken(text, start) {
  let index = start;
  if (index >= text.length || !isDigit(text[index])) return -1;
  while (index < text.length && isDigit(text[index])) index += 1;
  const spaced = text[index] === " " ? index + 1 : index;
  const tail = text.slice(spaced).toLowerCase();
  for (const word of HOUSE_SUFFIXES) {
    if (!tail.startsWith(word)) continue;
    const end = spaced + word.length;
    if (boundary(text, end)) return end;
  }
  if (spaced < text.length && isLetter(text[spaced]) && boundary(text, spaced + 1)) {
    return spaced + 1;
  }
  return index;
}

function isHouseNumber(part) {
  const text = part.trim();
  if (!text) return false;
  let end = eatHouseToken(text, 0);
  if (end < 0) return false;
  if (end === text.length) return true;
  if (text[end] !== "-" && text[end] !== "/") return false;
  end += 1;
  if (text[end] === " ") end += 1;
  return eatHouseToken(text, end) === text.length;
}

function addressParts(address) {
  return String(address || "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

// `12, rue Example` becomes `12 rue Example`. A bare number stays its own segment.
function joinHouseNumbers(parts) {
  const joined = [];
  for (let index = 0; index < parts.length; index += 1) {
    const next = parts[index + 1];
    if (next && isHouseNumber(parts[index]) && !isHouseNumber(next)) {
      joined.push(`${parts[index]} ${next}`);
      index += 1;
    } else {
      joined.push(parts[index]);
    }
  }
  return joined;
}

function formatAddress(address) {
  return joinHouseNumbers(addressParts(address)).join(", ");
}

// Title skips a leading house number. The detail keeps that number on the street.
function labelAddress(address) {
  const parts = addressParts(address);
  if (!parts.length) return null;
  const joined = joinHouseNumbers(parts);
  const leadingNumber = isHouseNumber(parts[0]) && parts[1] && !isHouseNumber(parts[1]);
  const name = leadingNumber ? parts[1] : joined[0];
  const detailParts = joined[0] === name ? joined.slice(1) : joined;
  const detail = detailParts.join(", ");
  return { name, detail: detail || null };
}

export function place(address, geofence) {
  const raw = String(address || "").trim();
  const fence = String(geofence || "").trim();
  if (fence && raw && fence !== raw) return { name: fence, detail: formatAddress(raw) };
  if (fence) return { name: fence, detail: null };
  if (raw) return labelAddress(raw);
  return null;
}

export function makeFormat(options = {}) {
  const locale = options.locale || "fr";
  const timeZone = options.timeZone || "UTC";
  const hooks = options.format || {};
  const fr = locale.toLowerCase().startsWith("fr");
  const sep = fr ? NNBSP : " ";
  const strings = { ...stringsFor(locale), ...(options.strings || {}), ...(hooks.strings || {}) };
  const num = hooks.number || ((value, decimals) => (fr ? number(value, decimals) : intlNumber(locale, value, decimals)));
  const unitOf = hooks.unit || ((value, decimals, suffix) => {
    if (value == null || !Number.isFinite(Number(value))) return "—";
    return `${num(Number(value), decimals)}${sep}${suffix}`;
  });
  return {
    locale,
    timeZone,
    strings,
    sep,
    number: num,
    unit: unitOf,
    distance: hooks.distance || ((km) => (km == null || !Number.isFinite(Number(km)) ? "—" : unitOf(Number(km), Math.abs(km) < 100 ? 1 : 0, strings.km))),
    energy: hooks.energy || ((kwh) => unitOf(kwh, 1, strings.kwh)),
    percent: hooks.percent || ((value) => unitOf(value, 0, "%")),
    money: hooks.money || ((value) => unitOf(value, 2, strings.currency)),
    duration: hooks.duration || ((seconds) => duration(seconds, strings, sep)),
    clock: hooks.clock || ((iso) => clock(iso, timeZone)),
    dayLabel: hooks.dayLabel || ((iso, now) => dayLabel(iso, timeZone, now, strings, locale)),
    ago: hooks.ago || ((iso, now) => ago(iso, now, strings, sep)),
  };
}
