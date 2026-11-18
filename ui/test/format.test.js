import assert from "node:assert/strict";
import test from "node:test";

import { NNBSP, dayLabel, duration, makeFormat, number } from "../src/format.js";
import { stringsFor } from "../src/i18n.js";

test("french numbers match the console", () => {
  assert.equal(number(1234.56, 1), `1${NNBSP}234,6`);
  assert.equal(number(-3, 0), "−3");
  assert.equal(number(-0.01, 0), "0");
  const fmt = makeFormat({ locale: "fr" });
  assert.equal(fmt.distance(12.34), `12,3${NNBSP}km`);
  assert.equal(fmt.distance(312.4), `312${NNBSP}km`);
  assert.equal(fmt.distance(null), "—");
});

test("durations read naturally", () => {
  const strings = stringsFor("fr");
  assert.equal(duration(42, strings), `42${NNBSP}s`);
  assert.equal(duration(35 * 60, strings), `35${NNBSP}min`);
  assert.equal(duration(3900, strings), `1${NNBSP}h${NNBSP}05`);
  assert.equal(duration(null, strings), "—");
});

test("day labels are relative then long", () => {
  const strings = stringsFor("fr");
  const today = "2026-11-18T12:00:00Z";
  assert.equal(dayLabel(today, "UTC", today, strings, "fr"), "Aujourd'hui");
  assert.equal(dayLabel("2026-11-17T12:00:00Z", "UTC", today, strings, "fr"), "Hier");
  assert.equal(dayLabel("2026-11-16T12:00:00Z", "UTC", today, strings, "fr"), "Lundi 16 novembre");
  assert.equal(dayLabel("2025-02-01T12:00:00Z", "UTC", today, strings, "fr"), "Samedi 1 février 2025");
});

test("a format hook replaces the built-in distance", () => {
  const fmt = makeFormat({
    locale: "en",
    format: { distance: (km) => `${km} miles` },
  });
  assert.equal(fmt.distance(3), "3 miles");
  assert.equal(fmt.strings.unavailableImage, "Vehicle image unavailable");
});
