// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: MIT OR Apache-2.0

import { TlElement, contextOf, h } from "./dom.js";
import { photo, sheet } from "./styles.js";

export class TlCarImage extends TlElement {
  constructor() {
    super(sheet(photo));
  }

  static get observedAttributes() {
    return [...TlElement.observedAttributes, "image-url", "alt", "unavailable"];
  }

  set imageUrl(value) {
    if (value) this.setAttribute("image-url", value);
    else this.removeAttribute("image-url");
  }

  get imageUrl() {
    return this.getAttribute("image-url");
  }

  paint(root) {
    const fmt = contextOf(this);
    const url = this.getAttribute("image-url") || "";
    const message = this.getAttribute("unavailable") || fmt.strings.unavailableImage;
    const alt = this.getAttribute("alt") || message;
    this.classList.remove("ready");
    const stage = h("div", { class: "stage" }, root);
    const placeholder = h("p", { class: "placeholder", text: message }, stage);
    placeholder.setAttribute("role", "img");
    placeholder.setAttribute("aria-label", message);
    if (!url) return;
    const img = h("img", { alt, decoding: "async" }, stage);
    const show = () => {
      if (img.naturalWidth > 0) this.classList.add("ready");
    };
    const hide = () => this.classList.remove("ready");
    img.addEventListener("load", show);
    img.addEventListener("error", hide);
    img.src = url;
    if (img.complete) {
      if (img.naturalWidth > 0) show();
      else hide();
    }
  }
}
