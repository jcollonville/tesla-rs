// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only
//
// Interactive basemap for a drive or a charge. Tiles come from this server.
// The chart cursor calls teslaMapMove with the same sample index charts.js uses.

(function () {
  if (!window.maplibregl) return;
  maplibregl.setWorkerUrl("/static/maplibre-gl-csp-worker.js");

  var maps = {};

  function theme() {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "positron";
  }

  function color(name, fallback) {
    var value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return value || fallback;
  }

  function pairs(list) {
    var out = [];
    (list || []).forEach(function (pair) {
      if (pair && pair.length === 2 && isFinite(pair[0]) && isFinite(pair[1])) out.push(pair);
    });
    return out;
  }

  function lineFeature(coordinates) {
    return {
      type: "Feature",
      properties: {},
      geometry: { type: "LineString", coordinates: coordinates },
    };
  }

  function pointFeature(coordinates, kind) {
    return {
      type: "Feature",
      properties: { kind: kind },
      geometry: { type: "Point", coordinates: coordinates },
    };
  }

  function emptyCollection() {
    return { type: "FeatureCollection", features: [] };
  }

  function paint(map, spec) {
    var accent = color("--accent", "#3e6ae1");
    var text = color("--text", "#171a20");
    var surface = color("--surface", "#ffffff");
    var line = pairs(spec.cursor);
    var point = spec.point && spec.point.length === 2 ? spec.point : null;
    if (line.length >= 2) {
      map.addSource("route", { type: "geojson", data: lineFeature(line) });
      map.addLayer({
        id: "route-casing",
        type: "line",
        source: "route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": surface, "line-width": 7, "line-opacity": 0.92 },
      });
      map.addLayer({
        id: "route-line",
        type: "line",
        source: "route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": accent, "line-width": 4 },
      });
    }
    var ends = [];
    if (line.length >= 1) {
      ends.push(pointFeature(line[0], "start"));
      ends.push(pointFeature(line[line.length - 1], "end"));
    } else if (point) {
      ends.push(pointFeature(point, "end"));
    }
    map.addSource("ends", {
      type: "geojson",
      data: { type: "FeatureCollection", features: ends },
    });
    map.addLayer({
      id: "ends",
      type: "circle",
      source: "ends",
      paint: {
        "circle-radius": ["match", ["get", "kind"], "end", 8, 6.5],
        "circle-color": ["match", ["get", "kind"], "end", accent, surface],
        "circle-stroke-width": 3,
        "circle-stroke-color": ["match", ["get", "kind"], "end", surface, text],
      },
    });
    map.addSource("cursor", { type: "geojson", data: emptyCollection() });
    map.addLayer({
      id: "cursor",
      type: "circle",
      source: "cursor",
      paint: {
        "circle-radius": 7,
        "circle-color": text,
        "circle-stroke-width": 3,
        "circle-stroke-color": surface,
      },
    });
  }

  function show(entry, index) {
    var source = entry.map.getSource("cursor");
    if (!source) return;
    var pair = entry.cursor[index];
    if (!pair || pair.length !== 2 || pair[0] < -180 || pair[0] > 180) {
      source.setData(emptyCollection());
      return;
    }
    source.setData({
      type: "FeatureCollection",
      features: [pointFeature(pair, "cursor")],
    });
  }

  function fit(map, spec) {
    var line = pairs(spec.cursor);
    var point = spec.point && spec.point.length === 2 ? spec.point : null;
    if (line.length >= 2) {
      var bounds = new maplibregl.LngLatBounds(line[0], line[0]);
      line.forEach(function (pair) {
        bounds.extend(pair);
      });
      map.fitBounds(bounds, { padding: 48, maxZoom: 14, animate: false, duration: 0 });
    } else if (line.length === 1) {
      map.jumpTo({ center: line[0], zoom: 14 });
    } else if (point) {
      map.jumpTo({ center: point, zoom: 14 });
    }
  }

  function boot(el) {
    var spec;
    try {
      spec = JSON.parse(el.getAttribute("data-map") || "");
    } catch (err) {
      return;
    }
    if (!spec || (!spec.point && !(spec.cursor && spec.cursor.length))) return;
    var frame = el.parentNode;
    var map;
    try {
      map = new maplibregl.Map({
        container: el,
        style: "/tiles/p/styles/" + theme(),
        attributionControl: false,
        dragRotate: false,
        pitchWithRotate: false,
        maxZoom: 14,
        fadeDuration: 0,
        locale: {
          "NavigationControl.ZoomIn": "Zoom avant",
          "NavigationControl.ZoomOut": "Zoom arrière",
        },
      });
    } catch (err) {
      return;
    }
    map.addControl(new maplibregl.NavigationControl({ showCompass: false, visualizePitch: false }), "top-right");
    map.touchZoomRotate.disableRotation();
    var entry = { map: map, cursor: spec.cursor || [], spec: spec, frame: frame };
    var sync = el.getAttribute("data-sync");
    if (sync) maps[sync] = entry;
    map.on("load", function () {
      paint(map, spec);
      if (frame && frame.classList) frame.classList.add("map-live");
      var fitted = false;
      function layout(refit) {
        map.resize();
        if (refit && !fitted) {
          fit(map, spec);
          fitted = true;
        }
      }
      requestAnimationFrame(function () {
        requestAnimationFrame(function () {
          layout(true);
        });
      });
      if (window.ResizeObserver && frame) {
        new ResizeObserver(function () {
          map.resize();
        }).observe(frame);
      }
    });
    if (window.matchMedia) {
      var media = window.matchMedia("(prefers-color-scheme: dark)");
      var onTheme = function () {
        map.setStyle("/tiles/p/styles/" + theme());
        map.once("style.load", function () {
          paint(map, spec);
        });
      };
      if (media.addEventListener) media.addEventListener("change", onTheme);
      else if (media.addListener) media.addListener(onTheme);
    }
  }

  document.querySelectorAll(".basemap").forEach(boot);

  window.teslaMapMove = function (sync, index) {
    var entry = maps[sync];
    if (!entry || !entry.map.isStyleLoaded()) return;
    show(entry, index);
  };

  window.teslaMapHide = function (sync) {
    var entry = maps[sync];
    if (!entry) return;
    var source = entry.map.getSource("cursor");
    if (source) source.setData(emptyCollection());
  };
})();
