import assert from "node:assert/strict";
import test from "node:test";

import { NNBSP, dayLabel, duration, makeFormat, number, place } from "../src/format.js";
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

test("place titles use the street and details keep the house number", () => {
  const numbered = place("12, rue Example, Localité");
  assert.equal(numbered.name, "rue Example");
  assert.equal(numbered.detail, "12 rue Example, Localité");

  const suffixed = place("12 bis, rue Example");
  assert.equal(suffixed.name, "rue Example");
  assert.equal(suffixed.detail, "12 bis rue Example");

  const street = place("rue Example, Localité");
  assert.equal(street.name, "rue Example");
  assert.equal(street.detail, "Localité");

  const numberOnly = place("12");
  assert.equal(numberOnly.name, "12");
  assert.equal(numberOnly.detail, null);

  const locality = place("12, Localité");
  assert.equal(locality.name, "Localité");
  assert.equal(locality.detail, "12 Localité");

  const noStreet = place("Localité, Région");
  assert.equal(noStreet.name, "Localité");
  assert.equal(noStreet.detail, "Région");

  const named = place("Café Example, 12, rue Example, Localité");
  assert.equal(named.name, "Café Example");
  assert.equal(named.detail, "12 rue Example, Localité");

  const fenced = place("12, rue Example, Localité", "Maison");
  assert.equal(fenced.name, "Maison");
  assert.equal(fenced.detail, "12 rue Example, Localité");
});

test("a format hook replaces the built-in distance", () => {
  const fmt = makeFormat({
    locale: "en",
    format: { distance: (km) => `${km} miles` },
  });
  assert.equal(fmt.distance(3), "3 miles");
  assert.equal(fmt.strings.unavailableImage, "Vehicle image unavailable");
});
