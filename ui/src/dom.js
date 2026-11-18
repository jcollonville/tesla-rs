// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { makeFormat } from "./format.js";

const SVG = "http://www.w3.org/2000/svg";

export function h(tag, attrs, parent) {
  const node = document.createElement(tag);
  apply(node, attrs);
  if (parent) parent.appendChild(node);
  return node;
}

export function svgEl(tag, attrs, parent) {
  const node = document.createElementNS(SVG, tag);
  apply(node, attrs);
  if (parent) parent.appendChild(node);
  return node;
}

function apply(node, attrs) {
  if (!attrs) return;
  for (const [key, value] of Object.entries(attrs)) {
    if (value == null || value === false) continue;
    if (key === "class") node.setAttribute("class", value);
    else if (key === "text") node.textContent = String(value);
    else node.setAttribute(key, String(value));
  }
}

export function readData(el) {
  if (Object.prototype.hasOwnProperty.call(el, "_data")) return el._data;
  const node = el.querySelector(":scope > script[type='application/json']");
  if (!node || !node.textContent) return null;
  try {
    return JSON.parse(node.textContent);
  } catch {
    return null;
  }
}

export function adopt(root, css) {
  try {
    if ("adoptedStyleSheets" in root && typeof CSSStyleSheet === "function") {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(css);
      root.adoptedStyleSheets = [sheet];
      return;
    }
  } catch {
    /* jsdom and older engines fall through to a style element */
  }
  const style = document.createElement("style");
  style.textContent = css;
  root.appendChild(style);
}

export function contextOf(el) {
  const locale = el.getAttribute("locale")
    || el.closest("[locale]")?.getAttribute("locale")
    || document.documentElement?.lang
    || "fr";
  const timeZone = el.getAttribute("time-zone")
    || el.closest("[time-zone]")?.getAttribute("time-zone")
    || "UTC";
  return makeFormat({
    locale,
    timeZone,
    format: el._format || null,
    strings: el._strings || null,
  });
}

export function nowOf(el) {
  return el.getAttribute("now") || new Date().toISOString();
}

export class TlElement extends HTMLElement {
  constructor(css) {
    super();
    const root = this.attachShadow({ mode: "open" });
    adopt(root, css);
  }

  static get observedAttributes() {
    return ["locale", "time-zone", "theme", "now"];
  }

  connectedCallback() {
    this.render();
  }

  attributeChangedCallback() {
    if (this.isConnected && !this._rendering) this.render();
  }

  set data(value) {
    this._data = value;
    if (this.isConnected && !this._rendering) this.render();
  }

  get data() {
    return readData(this);
  }

  set format(value) {
    this._format = value || null;
    if (this.isConnected && !this._rendering) this.render();
  }

  get format() {
    return this._format || null;
  }

  set strings(value) {
    this._strings = value || null;
    if (this.isConnected && !this._rendering) this.render();
  }

  get strings() {
    return this._strings || null;
  }

  set locale(value) {
    this.setAttribute("locale", value ?? "");
  }

  get locale() {
    return this.getAttribute("locale");
  }

  set timeZone(value) {
    this.setAttribute("time-zone", value || "");
  }

  get timeZone() {
    return this.getAttribute("time-zone");
  }

  set theme(value) {
    if (value) this.setAttribute("theme", value);
    else this.removeAttribute("theme");
  }

  get theme() {
    return this.getAttribute("theme");
  }

  set now(value) {
    this.setAttribute("now", value || "");
  }

  get now() {
    return this.getAttribute("now");
  }

  render() {
    if (this._rendering) return;
    this._rendering = true;
    const root = this.shadowRoot;
    for (const node of [...root.childNodes]) {
      if (node.nodeName !== "STYLE") root.removeChild(node);
    }
    try {
      this.paint(root);
    } finally {
      this._rendering = false;
    }
  }

  paint() {}
}

export function emit(el, name, detail) {
  el.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));
}
