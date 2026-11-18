// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

// Interactive charts. The console still ships a static fallback; this module
// draws axes, tooltips, a synced cursor and drag-to-zoom. No inline style
// attributes: colour comes from classes, and the tooltip position is a CSSOM
// transform (allowed under a strict style-src).


  "use strict";

  var NS = "http://www.w3.org/2000/svg";
  var NNBSP = "\u202f";
  var motion = window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;
  var groups = {};
  var serial = 0;

  function still() {
    return motion && motion.matches;
  }

  function svg(name, attrs, parent) {
    var node = document.createElementNS(NS, name);
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        node.setAttribute(key, attrs[key]);
      });
    }
    if (parent) parent.appendChild(node);
    return node;
  }

  function html(name, className, parent) {
    var node = document.createElement(name);
    if (className) node.className = className;
    if (parent) parent.appendChild(node);
    return node;
  }

  function number(value, decimals) {
    if (value === null || value === undefined || !isFinite(value)) return "—";
    var text = Number(value).toLocaleString("fr-FR", {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals
    });
    return text.replace("-", "\u2212");
  }

  function withUnit(value, decimals, unit) {
    var text = number(value, decimals);
    if (text === "—" || !unit) return text;
    return text + NNBSP + unit;
  }

  function pad(value) {
    return value < 10 ? "0" + value : String(value);
  }

  function nice(span, count) {
    var raw = span / Math.max(1, count);
    var power = Math.pow(10, Math.floor(Math.log(raw) / Math.LN10));
    var unit = raw / power;
    var step = unit < 1.5 ? 1 : unit < 3 ? 2 : unit < 7 ? 5 : 10;
    return step * power;
  }

  function yScale(low, high, count, signed) {
    if (signed) {
      low = Math.min(low, 0);
      high = Math.max(high, 0);
    }
    if (!(high > low)) {
      high = low + 1;
      if (!signed) low = low - 1;
    }
    var step = nice(high - low, count);
    var lo = Math.floor(low / step) * step;
    var hi = Math.ceil(high / step) * step;
    var ticks = [];
    for (var value = lo; value <= hi + step / 2; value += step) ticks.push(Math.abs(value) < step / 1e6 ? 0 : value);
    return { lo: lo, hi: hi, ticks: ticks, step: step };
  }

  function decimalsFor(step) {
    if (step >= 1) return 0;
    return Math.min(3, Math.ceil(-Math.log(step) / Math.LN10));
  }

  function clockAt(axis, seconds, withSeconds) {
    var date = new Date(axis.origin_ms + seconds * 1000 + axis.offset_min * 60000);
    var text = pad(date.getUTCHours()) + ":" + pad(date.getUTCMinutes());
    return withSeconds ? text + ":" + pad(date.getUTCSeconds()) : text;
  }

  function elapsed(seconds) {
    var minutes = Math.round(seconds / 60);
    if (minutes < 60) return minutes + NNBSP + "min";
    return Math.floor(minutes / 60) + NNBSP + "h" + NNBSP + pad(minutes % 60);
  }

  function xLabel(axis, value, precise) {
    if (axis.kind === "time") return clockAt(axis, value, precise);
    return withUnit(value, axis.decimals || 0, axis.unit);
  }

  function timeTicks(axis, d0, d1, count) {
    var steps = [60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];
    var span = d1 - d0;
    var step = steps[steps.length - 1];
    for (var index = 0; index < steps.length; index++) {
      if (span / steps[index] <= count) {
        step = steps[index];
        break;
      }
    }
    var local = axis.origin_ms / 1000 + axis.offset_min * 60;
    var first = Math.ceil((local + d0) / step) * step - local;
    var ticks = [];
    for (var value = first; value <= d1; value += step) ticks.push(value);
    return ticks;
  }

  function numberTicks(d0, d1, count) {
    var step = nice(d1 - d0, count);
    var ticks = [];
    for (var value = Math.ceil(d0 / step) * step; value <= d1 + step / 1e6; value += step) ticks.push(value);
    return ticks;
  }

  function nearest(values, target) {
    var low = 0;
    var high = values.length - 1;
    while (high - low > 1) {
      var middle = (low + high) >> 1;
      if (values[middle] < target) low = middle;
      else high = middle;
    }
    return Math.abs(values[low] - target) <= Math.abs(values[high] - target) ? low : high;
  }

  function ease(progress) {
    return 1 - Math.pow(1 - progress, 3);
  }

  function animate(duration, step, done) {
    if (still()) {
      step(1);
      if (done) done();
      return;
    }
    var start = null;
    function frame(now) {
      if (start === null) start = now;
      var progress = Math.min(1, (now - start) / duration);
      step(ease(progress));
      if (progress < 1) window.requestAnimationFrame(frame);
      else if (done) done();
    }
    window.requestAnimationFrame(frame);
  }

  function Chart(figure, data) {
    this.figure = figure;
    this.data = data;
    this.id = "c" + ++serial;
    this.plot = figure.querySelector(".chart-plot");
    this.sync = figure.getAttribute("data-sync");
    this.active = -1;
    this.reveal = 0;
    if (data.kind === "line") {
      this.full = [data.t[0], data.t[data.t.length - 1]];
      this.view = this.full.slice();
    }
    if (!this.sync || !groups[this.sync]) figure.classList.add("chart-lead");
    this.build();
    if (this.sync) {
      (groups[this.sync] = groups[this.sync] || []).push(this);
      this.map = document.querySelector('svg.map[data-sync="' + this.sync + '"]');
      if (this.map) {
        try {
          this.mapXY = JSON.parse(this.map.getAttribute("data-xy") || "[]");
        } catch (err) {
          this.mapXY = [];
        }
        this.mapCursor = this.map.querySelector(".map-cursor");
      }
    }
  }

  Chart.prototype.build = function () {
    var self = this;
    this.figure.classList.add("chart-ready");
    this.root = svg("svg", { class: "chart-svg", role: "presentation", "aria-hidden": "true", focusable: "false" }, this.plot);
    this.tip = html("div", "chart-tooltip", this.plot);
    this.tip.setAttribute("aria-hidden", "true");
    this.live = html("p", "visually-hidden", this.figure);
    this.live.setAttribute("aria-live", "polite");
    if (this.data.kind === "line") {
      var tools = html("div", "chart-tools", this.figure);
      var reset = html("button", "ghost small", tools);
      reset.type = "button";
      reset.textContent = "Réinitialiser le zoom";
      reset.addEventListener("click", function () {
        self.zoomTo(self.full[0], self.full[1], true);
      });
    }
    if (this.data.series.length > 1) {
      var legend = html("p", "chart-legend", this.figure);
      this.data.series.forEach(function (series) {
        var item = html("span", "", legend);
        html("span", "tip-swatch tone-" + series.tone, item);
        item.appendChild(document.createTextNode(series.name));
      });
      this.figure.insertBefore(legend, this.plot);
    }
    this.bind();
    this.render();
    animate(700, function (progress) {
      self.reveal = progress;
      self.render();
    });
  };

  Chart.prototype.size = function () {
    var width = Math.max(240, Math.round(this.plot.clientWidth || 600));
    var height = width < 520 ? 170 : 200;
    if (this.data.kind === "bar") height += 16;
    return { width: width, height: height, left: 44, right: 10, top: 10, bottom: 24 };
  };

  Chart.prototype.render = function () {
    if (this.data.kind === "bar") this.renderBars();
    else this.renderLines();
  };

  Chart.prototype.renderLines = function () {
    var data = this.data;
    var box = this.size();
    var root = this.root;
    while (root.firstChild) root.removeChild(root.firstChild);
    root.setAttribute("viewBox", "0 0 " + box.width + " " + box.height);
    root.setAttribute("width", box.width);
    root.setAttribute("height", box.height);
    var plotW = box.width - box.left - box.right;
    var plotH = box.height - box.top - box.bottom;
    var d0 = this.view[0];
    var d1 = this.view[1] > d0 ? this.view[1] : d0 + 1;
    var t = data.t;
    var first = Math.max(0, nearest(t, d0) - 1);
    var last = Math.min(t.length - 1, nearest(t, d1) + 1);
    var low = Infinity;
    var high = -Infinity;
    var signed = false;
    data.series.forEach(function (series) {
      if (series.signed) signed = true;
      for (var index = first; index <= last; index++) {
        var value = series.values[index];
        if (value === null || value === undefined) continue;
        if (value < low) low = value;
        if (value > high) high = value;
      }
    });
    if (!isFinite(low)) {
      low = 0;
      high = 1;
    }
    var scale = yScale(low, high, box.height < 190 ? 3 : 4, signed);
    var tickDecimals = decimalsFor(scale.step);
    function px(value) {
      return box.left + ((value - d0) / (d1 - d0)) * plotW;
    }
    function py(value) {
      return box.top + plotH - ((value - scale.lo) / (scale.hi - scale.lo)) * plotH;
    }
    this.px = px;
    this.py = py;
    this.box = box;

    var axis = svg("g", { class: "chart-axis" }, root);
    scale.ticks.forEach(function (tick) {
      var y = Math.round(py(tick)) + 0.5;
      svg("line", { class: tick === 0 && signed ? "chart-zero" : "chart-grid-line", x1: box.left, x2: box.width - box.right, y1: y, y2: y }, axis);
      var label = svg("text", { class: "chart-tick", x: box.left - 8, y: y, "text-anchor": "end", "dominant-baseline": "middle" }, axis);
      label.textContent = number(tick, tickDecimals);
    });
    var ticks = data.x.kind === "time" ? timeTicks(data.x, d0, d1, Math.floor(plotW / 72)) : numberTicks(d0, d1, Math.floor(plotW / 72));
    ticks.forEach(function (tick) {
      var label = svg("text", { class: "chart-tick", x: px(tick), y: box.height - 6, "text-anchor": "middle" }, axis);
      label.textContent = data.x.kind === "time" ? clockAt(data.x, tick, false) : number(tick, data.x.decimals || 0);
    });

    var defs = svg("defs", null, root);
    var clip = svg("clipPath", { id: this.id + "-clip" }, defs);
    svg("rect", { x: box.left, y: 0, width: Math.max(0, plotW * this.reveal), height: box.height }, clip);
    var layer = svg("g", { "clip-path": "url(#" + this.id + "-clip)" }, root);
    var self = this;
    data.series.forEach(function (series, number) {
      var segments = [];
      var current = [];
      for (var index = first; index <= last; index++) {
        var value = series.values[index];
        if (value === null || value === undefined) {
          if (current.length) segments.push(current);
          current = [];
          continue;
        }
        current.push([px(t[index]), py(value)]);
      }
      if (current.length) segments.push(current);
      var line = "";
      var area = "";
      var base = signed ? py(0) : box.top + plotH;
      segments.forEach(function (points) {
        var path = points.map(function (point, index) {
          return (index ? "L" : "M") + point[0].toFixed(1) + " " + point[1].toFixed(1);
        }).join("");
        line += path;
        if (series.area && points.length > 1) {
          area += path + "L" + points[points.length - 1][0].toFixed(1) + " " + base.toFixed(1) + "L" + points[0][0].toFixed(1) + " " + base.toFixed(1) + "Z";
        }
      });
      var tone = "tone-" + series.tone;
      if (area) {
        var gradientId = self.id + "-g" + number;
        var gradient = svg("linearGradient", { id: gradientId, class: tone, x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
        svg("stop", { offset: 0, "stop-color": "currentColor", "stop-opacity": signed ? 0.22 : 0.26 }, gradient);
        svg("stop", { offset: 1, "stop-color": "currentColor", "stop-opacity": 0.02 }, gradient);
        svg("path", { class: "chart-area", d: area, fill: "url(#" + gradientId + ")" }, layer);
      }
      svg("path", { class: "chart-line " + tone, d: line }, layer);
    });

    var cursor = svg("g", { class: "chart-cursor" }, root);
    this.crosshair = svg("line", { class: "chart-crosshair", x1: -10, x2: -10, y1: box.top, y2: box.top + plotH }, cursor);
    this.dots = data.series.map(function (series) {
      return svg("circle", { class: "chart-dot tone-" + series.tone, r: 4, cx: -10, cy: -10 }, cursor);
    });
    this.brush = svg("rect", { class: "chart-brush", x: 0, y: box.top, width: 0, height: plotH }, root);
    this.figure.classList.toggle("zoomed", this.view[0] > this.full[0] || this.view[1] < this.full[1]);
    if (this.active >= 0) this.showIndex(this.active, false);
  };

  Chart.prototype.renderBars = function () {
    var data = this.data;
    var box = this.size();
    box.bottom = 30;
    var root = this.root;
    while (root.firstChild) root.removeChild(root.firstChild);
    root.setAttribute("viewBox", "0 0 " + box.width + " " + box.height);
    root.setAttribute("width", box.width);
    root.setAttribute("height", box.height);
    var plotW = box.width - box.left - box.right;
    var plotH = box.height - box.top - box.bottom;
    var count = data.labels.length;
    var totals = [];
    var high = 0;
    for (var index = 0; index < count; index++) {
      var sum = 0;
      var top = 0;
      data.series.forEach(function (series) {
        var value = series.values[index] || 0;
        sum += value;
        if (value > top) top = value;
      });
      totals.push(sum);
      high = Math.max(high, data.stacked ? sum : top);
    }
    var scale = yScale(0, high || 1, box.height < 200 ? 3 : 4, false);
    var tickDecimals = decimalsFor(scale.step);
    function py(value) {
      return box.top + plotH - ((value - scale.lo) / (scale.hi - scale.lo)) * plotH;
    }
    var axis = svg("g", { class: "chart-axis" }, root);
    scale.ticks.forEach(function (tick) {
      var y = Math.round(py(tick)) + 0.5;
      svg("line", { class: "chart-grid-line", x1: box.left, x2: box.width - box.right, y1: y, y2: y }, axis);
      var label = svg("text", { class: "chart-tick", x: box.left - 8, y: y, "text-anchor": "end", "dominant-baseline": "middle" }, axis);
      label.textContent = number(tick, tickDecimals);
    });
    var band = plotW / Math.max(1, count);
    var barW = Math.max(3, Math.min(42, band * 0.62));
    var every = Math.max(1, Math.ceil(52 / band));
    var bars = svg("g", { class: "chart-bars" }, root);
    var self = this;
    this.barGroups = [];
    for (var slot = 0; slot < count; slot++) {
      var centre = box.left + band * slot + band / 2;
      var group = svg("g", { class: "chart-bar-group" }, bars);
      var base = py(0);
      var series = data.series;
      if (data.stacked) {
        var stack = 0;
        series.forEach(function (one, number) {
          var value = (one.values[slot] || 0) * self.reveal;
          if (value <= 0) return;
          var y0 = py(stack);
          var y1 = py(stack + value);
          stack += value;
          var isTop = number === series.length - 1 || series.slice(number + 1).every(function (rest) { return !(rest.values[slot] > 0); });
          svg("rect", { class: "chart-bar tone-" + one.tone, x: (centre - barW / 2).toFixed(1), y: y1.toFixed(1), width: barW.toFixed(1), height: Math.max(0.5, y0 - y1).toFixed(1), rx: isTop ? Math.min(4, barW / 4) : 0 }, group);
        });
      } else {
        var inner = barW / series.length;
        series.forEach(function (one, number) {
          var value = one.values[slot];
          if (value === null || value === undefined) return;
          var y = py(value * self.reveal);
          svg("rect", { class: "chart-bar tone-" + one.tone, x: (centre - barW / 2 + inner * number).toFixed(1), y: Math.min(y, base).toFixed(1), width: Math.max(1, inner - 1).toFixed(1), height: Math.max(0.5, Math.abs(base - y)).toFixed(1), rx: Math.min(4, inner / 4) }, group);
        });
      }
      var hit = svg("rect", { class: "chart-bar-hit", x: box.left + band * slot, y: box.top, width: band, height: plotH + box.bottom }, group);
      hit.setAttribute("data-index", slot);
      this.barGroups.push(group);
      if (slot % every === 0) {
        var label = svg("text", { class: "chart-tick", x: centre, y: box.height - 10, "text-anchor": "middle" }, axis);
        label.textContent = data.labels[slot];
      }
    }
    this.barBand = band;
    this.box = box;
    this.totals = totals;
    if (this.active >= 0) this.showBar(this.active, false);
  };

  Chart.prototype.bind = function () {
    var self = this;
    var plot = this.plot;
    var drag = null;

    function localX(event) {
      var rect = plot.getBoundingClientRect();
      var scale = self.box ? self.box.width / rect.width : 1;
      return (event.clientX - rect.left) * scale;
    }

    function valueAt(x) {
      var box = self.box;
      var ratio = (x - box.left) / (box.width - box.left - box.right);
      return self.view[0] + Math.max(0, Math.min(1, ratio)) * (self.view[1] - self.view[0]);
    }

    plot.addEventListener("pointermove", function (event) {
      var x = localX(event);
      if (self.data.kind === "bar") {
        var slot = Math.floor((x - self.box.left) / self.barBand);
        if (slot >= 0 && slot < self.data.labels.length) self.showBar(slot, true);
        return;
      }
      if (drag) {
        drag.current = x;
        var left = Math.max(self.box.left, Math.min(drag.start, x));
        var right = Math.min(self.box.width - self.box.right, Math.max(drag.start, x));
        self.brush.setAttribute("x", left);
        self.brush.setAttribute("width", Math.max(0, right - left));
      }
      var index = nearest(self.data.t, valueAt(x));
      self.broadcast(index);
    });

    plot.addEventListener("pointerleave", function () {
      if (drag) return;
      self.clearGroup();
    });

    plot.addEventListener("pointerdown", function (event) {
      if (self.data.kind !== "line" || event.button !== 0) return;
      drag = { start: localX(event), current: localX(event), pointer: event.pointerId };
      if (plot.setPointerCapture) plot.setPointerCapture(event.pointerId);
    });

    function finish(event) {
      if (!drag) return;
      var start = drag.start;
      var end = drag.current;
      if (plot.releasePointerCapture && drag.pointer !== undefined) {
        try {
          plot.releasePointerCapture(drag.pointer);
        } catch (err) {
          /* pointer already released */
        }
      }
      drag = null;
      self.brush.setAttribute("width", 0);
      if (Math.abs(end - start) > 12) {
        var a = valueAt(Math.min(start, end));
        var b = valueAt(Math.max(start, end));
        self.zoomTo(a, b, true);
      }
      if (event && event.type === "pointercancel") self.clearGroup();
    }
    plot.addEventListener("pointerup", finish);
    plot.addEventListener("pointercancel", finish);

    plot.addEventListener("dblclick", function () {
      if (self.data.kind === "line") self.zoomTo(self.full[0], self.full[1], true);
    });

    plot.addEventListener("click", function (event) {
      if (self.data.kind !== "bar" || !self.data.hrefs) return;
      var target = event.target;
      var slot = target && target.getAttribute ? target.getAttribute("data-index") : null;
      if (slot !== null && self.data.hrefs[slot]) window.location.href = self.data.hrefs[slot];
    });

    this.figure.addEventListener("keydown", function (event) {
      var size = self.data.kind === "bar" ? self.data.labels.length : self.data.t.length;
      var step = event.shiftKey ? 10 : 1;
      var index = self.active;
      if (index < 0) {
        index = self.data.kind === "bar" ? size - 1 : nearest(self.data.t, (self.view[0] + self.view[1]) / 2);
      }
      var handled = true;
      switch (event.key) {
        case "ArrowRight":
          index = Math.min(size - 1, index + step);
          break;
        case "ArrowLeft":
          index = Math.max(0, index - step);
          break;
        case "Home":
          index = 0;
          break;
        case "End":
          index = size - 1;
          break;
        case "+":
        case "=":
          if (self.data.kind !== "line") return;
          self.zoomAround(self.data.t[index], 0.5);
          break;
        case "-":
        case "_":
          if (self.data.kind !== "line") return;
          self.zoomAround(self.data.t[index], 2);
          break;
        case "0":
        case "Escape":
          if (self.data.kind === "line") self.zoomTo(self.full[0], self.full[1], true);
          self.clearGroup();
          return;
        case "Enter":
          if (self.data.kind === "bar" && self.data.hrefs && self.data.hrefs[index]) {
            window.location.href = self.data.hrefs[index];
          }
          return;
        default:
          handled = false;
      }
      if (!handled) return;
      event.preventDefault();
      if (self.data.kind === "bar") self.showBar(index, true);
      else {
        var t = self.data.t[index];
        if (t < self.view[0] || t > self.view[1]) {
          var span = self.view[1] - self.view[0];
          var from = Math.max(self.full[0], Math.min(self.full[1] - span, t - span / 2));
          self.zoomTo(from, from + span, true);
        }
        self.broadcast(index);
      }
      self.announce(index);
    });

    this.figure.addEventListener("blur", function () {
      self.clearGroup();
    });

    if (window.ResizeObserver) {
      var width = 0;
      new ResizeObserver(function () {
        var next = Math.round(plot.clientWidth);
        if (next !== width) {
          width = next;
          self.render();
        }
      }).observe(plot);
    } else {
      window.addEventListener("resize", function () {
        self.render();
      });
    }
  };

  Chart.prototype.zoomAround = function (centre, factor) {
    var span = (this.view[1] - this.view[0]) * factor;
    var full = this.full[1] - this.full[0];
    span = Math.max(Math.min(span, full), full / 200);
    var from = Math.max(this.full[0], Math.min(this.full[1] - span, centre - span / 2));
    this.zoomTo(from, from + span, true);
  };

  Chart.prototype.zoomTo = function (from, to, spread) {
    if (this.data.kind !== "line") return;
    var minimum = (this.full[1] - this.full[0]) / 200;
    if (to - from < minimum) {
      var middle = (from + to) / 2;
      from = middle - minimum / 2;
      to = middle + minimum / 2;
    }
    from = Math.max(this.full[0], from);
    to = Math.min(this.full[1], to);
    var self = this;
    var start = this.view.slice();
    animate(320, function (progress) {
      self.view = [start[0] + (from - start[0]) * progress, start[1] + (to - start[1]) * progress];
      self.render();
    });
    if (spread && this.sync && this.data.x.kind === "time") {
      (groups[this.sync] || []).forEach(function (other) {
        if (other !== self && other.data.kind === "line" && other.data.x.kind === "time") other.zoomTo(from, to, false);
      });
    }
  };

  Chart.prototype.broadcast = function (index) {
    var t = this.data.t[index];
    var self = this;
    this.showIndex(index, true);
    if (this.sync) {
      (groups[this.sync] || []).forEach(function (other) {
        if (other !== self && other.data.kind === "line") other.showIndex(nearest(other.data.t, t), false);
      });
    }
    this.moveMap(index);
  };

  Chart.prototype.clearGroup = function () {
    var members = this.sync ? groups[this.sync] || [this] : [this];
    members.forEach(function (member) {
      member.clear();
    });
    if (this.mapCursor) this.mapCursor.classList.remove("on");
    if (this.sync && window.teslaMapHide) window.teslaMapHide(this.sync);
  };

  Chart.prototype.clear = function () {
    this.active = -1;
    this.figure.classList.remove("chart-on");
    if (this.barGroups) {
      this.barGroups.forEach(function (group) {
        group.querySelectorAll(".chart-bar").forEach(function (bar) {
          bar.classList.remove("on");
        });
      });
    }
  };

  Chart.prototype.moveMap = function (index) {
    if (this.mapCursor && this.mapXY && this.mapXY[index] && this.mapXY[index][0] >= -50) {
      var point = this.mapXY[index];
      this.mapCursor.setAttribute("cx", point[0]);
      this.mapCursor.setAttribute("cy", point[1]);
      this.mapCursor.classList.add("on");
    }
    if (this.sync && window.teslaMapMove) window.teslaMapMove(this.sync, index);
  };

  Chart.prototype.showIndex = function (index, withTip) {
    var data = this.data;
    var t = data.t[index];
    if (t === undefined || !this.px) return;
    this.active = index;
    var x = this.px(t);
    var inside = t >= this.view[0] && t <= this.view[1];
    this.crosshair.setAttribute("x1", x);
    this.crosshair.setAttribute("x2", x);
    var self = this;
    var rows = [];
    data.series.forEach(function (series, number) {
      var value = series.values[index];
      var dot = self.dots[number];
      if (value === null || value === undefined || !inside) {
        dot.setAttribute("cx", -10);
        return;
      }
      dot.setAttribute("cx", x);
      dot.setAttribute("cy", self.py(value));
      rows.push([series, value]);
    });
    this.figure.classList.toggle("chart-on", inside);
    this.fillTip(xLabel(data.x, t, true) + (data.x.kind === "time" && t > 0 ? " · +" + elapsed(t) : ""), rows, x, withTip);
  };

  Chart.prototype.showBar = function (slot, withTip) {
    var data = this.data;
    this.active = slot;
    this.barGroups.forEach(function (group, index) {
      group.querySelectorAll(".chart-bar").forEach(function (bar) {
        bar.classList.toggle("on", index === slot);
      });
    });
    var rows = data.series.map(function (series) {
      return [series, series.values[slot]];
    });
    if (data.stacked && data.series.length > 1) {
      rows.push([{ name: "Total", unit: data.series[0].unit, decimals: data.series[0].decimals, tone: "" }, this.totals[slot]]);
    }
    this.figure.classList.add("chart-on");
    var x = this.box.left + this.barBand * slot + this.barBand / 2;
    this.fillTip((data.titles && data.titles[slot]) || data.labels[slot], rows, x, withTip);
  };

  Chart.prototype.fillTip = function (head, rows, x, visible) {
    var tip = this.tip;
    while (tip.firstChild) tip.removeChild(tip.firstChild);
    if (!visible) {
      tip.classList.add("visually-hidden");
      return;
    }
    tip.classList.remove("visually-hidden");
    html("div", "tip-head", tip).textContent = head;
    rows.forEach(function (row) {
      var line = html("div", "tip-row", tip);
      if (row[0].tone) html("span", "tip-swatch tone-" + row[0].tone, line);
      var name = html("span", "", line);
      name.textContent = row[0].name;
      var value = html("b", "", line);
      value.textContent = withUnit(row[1], row[0].decimals, row[0].unit);
    });
    var plotWidth = this.plot.clientWidth || 1;
    var ratio = plotWidth / this.box.width;
    var left = x * ratio + 14;
    var width = tip.offsetWidth || 140;
    if (left + width > plotWidth) left = x * ratio - width - 14;
    tip.style.transform = "translate(" + Math.max(0, Math.round(left)) + "px, 4px)";
  };

  Chart.prototype.announce = function (index) {
    var data = this.data;
    var head;
    var parts = [];
    if (data.kind === "bar") {
      head = (data.titles && data.titles[index]) || data.labels[index];
      data.series.forEach(function (series) {
        parts.push(series.name + " " + withUnit(series.values[index], series.decimals, series.unit));
      });
    } else {
      head = xLabel(data.x, data.t[index], false);
      data.series.forEach(function (series) {
        parts.push(series.name + " " + withUnit(series.values[index], series.decimals, series.unit));
      });
    }
    this.live.textContent = head + " : " + parts.join(", ");
  };

  function mountCharts(root) {
    var scope = root || document;
    var figures = scope.querySelectorAll("figure.chart[data-chart]");
    Array.prototype.forEach.call(figures, function (figure) {
      if (figure.closest && figure.closest("tl-chart")) return;
      var data;
      try {
        data = JSON.parse(figure.getAttribute("data-chart"));
      } catch (err) {
        return;
      }
      mountChart(figure, data);
    });
  }

  function mountChart(figure, data) {
    if (!figure || !data || !data.series || !data.series.length) return null;
    if (data.kind === "line" && (!data.t || data.t.length < 2)) return null;
    if (data.kind === "bar" && (!data.labels || !data.labels.length)) return null;
    return new Chart(figure, data);
  }

export { mountChart, mountCharts };
