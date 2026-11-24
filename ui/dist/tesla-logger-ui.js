var ut = (t) => {
  throw TypeError(t);
};
var Pt = (t, e, r) => e.has(t) || ut("Cannot " + r);
var dt = (t, e, r) => e.has(t) ? ut("Cannot add the same private member more than once") : e instanceof WeakSet ? e.add(t) : e.set(t, r);
var R = (t, e, r) => (Pt(t, e, "access private method"), r);
const jt = {
  today: "Aujourd'hui",
  yesterday: "Hier",
  days: ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
  months: [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre"
  ],
  shortMonths: ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
  unavailableImage: "Image du véhicule indisponible",
  unknownPlace: "Lieu inconnu",
  unknownPosition: "Position inconnue",
  enRoute: "En route",
  inProgress: "en cours",
  battery: "Batterie",
  range: "Autonomie",
  energy: "Énergie",
  efficiency: "Efficacité",
  maxPower: "Puissance max",
  cost: "Coût",
  drives: "Trajets",
  distance: "Distance",
  behindWheel: "Au volant",
  driving: "En route",
  charging: "En charge",
  plugged: "Branchée",
  parked: "Garée",
  asleep: "Endormie",
  offline: "Hors ligne",
  updating: "Mise à jour",
  unknown: "État inconnu",
  justNow: "à l'instant",
  never: "jamais",
  agoPrefix: "il y a",
  second: "s",
  minute: "min",
  hour: "h",
  day: "j",
  km: "km",
  kwh: "kWh",
  currency: "€",
  vehicle: "Voiture"
}, Ut = {
  today: "Today",
  yesterday: "Yesterday",
  days: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
  months: [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December"
  ],
  shortMonths: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
  unavailableImage: "Vehicle image unavailable",
  unknownPlace: "Unknown place",
  unknownPosition: "Unknown position",
  enRoute: "Driving",
  inProgress: "in progress",
  battery: "Battery",
  range: "Range",
  energy: "Energy",
  efficiency: "Efficiency",
  maxPower: "Max power",
  cost: "Cost",
  drives: "Drives",
  distance: "Distance",
  behindWheel: "Driving time",
  driving: "Driving",
  charging: "Charging",
  plugged: "Plugged in",
  parked: "Parked",
  asleep: "Asleep",
  offline: "Offline",
  updating: "Updating",
  unknown: "Unknown",
  justNow: "just now",
  never: "never",
  agoPrefix: "ago",
  second: "s",
  minute: "min",
  hour: "h",
  day: "d",
  km: "km",
  kwh: "kWh",
  currency: "€",
  vehicle: "Car"
};
function Bt(t) {
  return String(t || "fr").toLowerCase().startsWith("fr") ? jt : Ut;
}
const at = " ";
function It(t, e) {
  if (!Number.isFinite(t)) return "—";
  const r = Math.abs(t).toFixed(e), [i, a] = r.split(".");
  let n = "";
  for (let s = 0; s < i.length; s += 1)
    s > 0 && (i.length - s) % 3 === 0 && (n += at), n += i[s];
  const o = t < 0 && [...r].some((s) => s >= "1" && s <= "9") ? "−" : "";
  return a ? `${o}${n},${a}` : o + n;
}
function Ot(t, e, r) {
  return Number.isFinite(e) ? new Intl.NumberFormat(t, {
    minimumFractionDigits: r,
    maximumFractionDigits: r
  }).format(e) : "—";
}
function Wt(t, e, r = at) {
  if (t == null || !Number.isFinite(Number(t))) return "—";
  const i = Math.max(0, Math.trunc(Number(t)));
  if (i < 60) return `${i}${r}${e.second}`;
  const a = Math.trunc((i + 30) / 60);
  if (a < 60) return `${a}${r}${e.minute}`;
  const n = Math.trunc(a / 60), l = a % 60;
  return n >= 24 ? `${Math.trunc(n / 24)}${r}${e.day} ${n % 24}${r}${e.hour}` : `${n}${r}${e.hour}${r}${String(l).padStart(2, "0")}`;
}
function st(t, e) {
  return t > 0 && e > 0.05 ? t * 1e3 / e : null;
}
function Rt(t, e) {
  var l, o;
  const r = new Date(t);
  if (Number.isNaN(r.getTime())) return "—";
  const i = new Intl.DateTimeFormat("en-GB", {
    timeZone: e || "UTC",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23"
  }).formatToParts(r);
  let a = ((l = i.find((s) => s.type === "hour")) == null ? void 0 : l.value) || "00";
  const n = ((o = i.find((s) => s.type === "minute")) == null ? void 0 : o.value) || "00";
  return a === "24" && (a = "00"), `${a.padStart(2, "0")}:${n.padStart(2, "0")}`;
}
function ht(t, e) {
  const r = new Date(t);
  if (Number.isNaN(r.getTime())) return null;
  const i = new Intl.DateTimeFormat("en-US", {
    timeZone: e || "UTC",
    year: "numeric",
    month: "2-digit",
    day: "2-digit"
  }).formatToParts(r);
  return {
    y: Number(i.find((a) => a.type === "year").value),
    m: Number(i.find((a) => a.type === "month").value),
    d: Number(i.find((a) => a.type === "day").value)
  };
}
function ft(t, e) {
  return t && e && t.y === e.y && t.m === e.m && t.d === e.d;
}
function Gt(t, e) {
  const r = new Date(Date.UTC(t.y, t.m - 1, t.d));
  return r.setUTCDate(r.getUTCDate() + e), { y: r.getUTCFullYear(), m: r.getUTCMonth() + 1, d: r.getUTCDate() };
}
function Ht(t) {
  return (new Date(Date.UTC(t.y, t.m - 1, t.d)).getUTCDay() + 6) % 7;
}
function qt(t, e) {
  if (!t) return "";
  const [r, ...i] = [...t];
  return r.toLocaleUpperCase(e) + i.join("");
}
function Xt(t, e, r) {
  const i = r.days[Ht(t)], a = r.months[t.m - 1];
  return e ? `${i} ${t.d} ${a} ${t.y}` : `${i} ${t.d} ${a}`;
}
function Yt(t, e, r, i, a) {
  const n = ht(t, e), l = ht(r || (/* @__PURE__ */ new Date()).toISOString(), e);
  return !n || !l ? "—" : ft(n, l) ? i.today : ft(n, Gt(l, -1)) ? i.yesterday : qt(Xt(n, n.y !== l.y, i), a);
}
function Jt(t, e, r, i = at) {
  if (!t) return r.never;
  const a = new Date(t).getTime(), n = new Date(e || Date.now()).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(n)) return r.never;
  const l = Math.trunc((n - a) / 1e3);
  if (l < 45) return r.justNow;
  const o = Math.trunc((l + 30) / 60);
  if (o < 60) return `${r.agoPrefix} ${o}${i}${r.minute}`;
  const s = Math.trunc(o / 60);
  return s < 48 ? `${r.agoPrefix} ${s}${i}${r.hour}` : `${r.agoPrefix} ${Math.trunc(s / 24)}${i}${r.day}`;
}
const Vt = ["quater", "bis", "ter"];
function pt(t) {
  return t >= "0" && t <= "9";
}
function Zt(t) {
  return t >= "a" && t <= "z" || t >= "A" && t <= "Z";
}
function gt(t, e) {
  const r = t[e];
  return r == null || r === " " || r === "-" || r === "/";
}
function mt(t, e) {
  let r = e;
  if (r >= t.length || !pt(t[r])) return -1;
  for (; r < t.length && pt(t[r]); ) r += 1;
  const i = t[r] === " " ? r + 1 : r, a = t.slice(i).toLowerCase();
  for (const n of Vt) {
    if (!a.startsWith(n)) continue;
    const l = i + n.length;
    if (gt(t, l)) return l;
  }
  return i < t.length && Zt(t[i]) && gt(t, i + 1) ? i + 1 : r;
}
function et(t) {
  const e = t.trim();
  if (!e) return !1;
  let r = mt(e, 0);
  return r < 0 ? !1 : r === e.length ? !0 : e[r] !== "-" && e[r] !== "/" ? !1 : (r += 1, e[r] === " " && (r += 1), mt(e, r) === e.length);
}
function xt(t) {
  return String(t || "").split(",").map((e) => e.trim()).filter(Boolean);
}
function yt(t) {
  const e = [];
  for (let r = 0; r < t.length; r += 1) {
    const i = t[r + 1];
    i && et(t[r]) && !et(i) ? (e.push(`${t[r]} ${i}`), r += 1) : e.push(t[r]);
  }
  return e;
}
function Qt(t) {
  return yt(xt(t)).join(", ");
}
function Kt(t) {
  const e = xt(t);
  if (!e.length) return null;
  const r = yt(e), a = et(e[0]) && e[1] && !et(e[1]) ? e[1] : r[0], l = (r[0] === a ? r.slice(1) : r).join(", ");
  return { name: a, detail: l || null };
}
function G(t, e) {
  const r = String(t || "").trim(), i = String(e || "").trim();
  return i && r && i !== r ? { name: i, detail: Qt(r) } : i ? { name: i, detail: null } : r ? Kt(r) : null;
}
function te(t = {}) {
  const e = t.locale || "fr", r = t.timeZone || "UTC", i = t.format || {}, a = e.toLowerCase().startsWith("fr"), n = a ? at : " ", l = { ...Bt(e), ...t.strings || {}, ...i.strings || {} }, o = i.number || ((c, d) => a ? It(c, d) : Ot(e, c, d)), s = i.unit || ((c, d, h) => c == null || !Number.isFinite(Number(c)) ? "—" : `${o(Number(c), d)}${n}${h}`);
  return {
    locale: e,
    timeZone: r,
    strings: l,
    sep: n,
    number: o,
    unit: s,
    distance: i.distance || ((c) => c == null || !Number.isFinite(Number(c)) ? "—" : s(Number(c), Math.abs(c) < 100 ? 1 : 0, l.km)),
    energy: i.energy || ((c) => s(c, 1, l.kwh)),
    percent: i.percent || ((c) => s(c, 0, "%")),
    money: i.money || ((c) => s(c, 2, l.currency)),
    duration: i.duration || ((c) => Wt(c, l, n)),
    clock: i.clock || ((c) => Rt(c, r)),
    dayLabel: i.dayLabel || ((c, d) => Yt(c, r, d, l, e)),
    ago: i.ago || ((c, d) => Jt(c, d, l, n))
  };
}
const ee = "http://www.w3.org/2000/svg";
function u(t, e, r) {
  const i = document.createElement(t);
  return wt(i, e), r && r.appendChild(i), i;
}
function x(t, e, r) {
  const i = document.createElementNS(ee, t);
  return wt(i, e), r && r.appendChild(i), i;
}
function wt(t, e) {
  if (e)
    for (const [r, i] of Object.entries(e))
      i == null || i === !1 || (r === "class" ? t.setAttribute("class", i) : r === "text" ? t.textContent = String(i) : t.setAttribute(r, String(i)));
}
function j(t) {
  if (Object.prototype.hasOwnProperty.call(t, "_data")) return t._data;
  const e = t.querySelector(":scope > script[type='application/json']");
  if (!e || !e.textContent) return null;
  try {
    return JSON.parse(e.textContent);
  } catch {
    return null;
  }
}
function re(t, e) {
  try {
    if ("adoptedStyleSheets" in t && typeof CSSStyleSheet == "function") {
      const i = new CSSStyleSheet();
      i.replaceSync(e), t.adoptedStyleSheets = [i];
      return;
    }
  } catch {
  }
  const r = document.createElement("style");
  r.textContent = e, t.appendChild(r);
}
function I(t) {
  var i, a, n;
  const e = t.getAttribute("locale") || ((i = t.closest("[locale]")) == null ? void 0 : i.getAttribute("locale")) || ((a = document.documentElement) == null ? void 0 : a.lang) || "fr", r = t.getAttribute("time-zone") || ((n = t.closest("[time-zone]")) == null ? void 0 : n.getAttribute("time-zone")) || "UTC";
  return te({
    locale: e,
    timeZone: r,
    format: t._format || null,
    strings: t._strings || null
  });
}
function ot(t) {
  return t.getAttribute("now") || (/* @__PURE__ */ new Date()).toISOString();
}
class _ extends HTMLElement {
  constructor(e) {
    super();
    const r = this.attachShadow({ mode: "open" });
    re(r, e);
  }
  static get observedAttributes() {
    return ["locale", "time-zone", "theme", "now"];
  }
  connectedCallback() {
    this.render();
  }
  attributeChangedCallback() {
    this.isConnected && !this._rendering && this.render();
  }
  set data(e) {
    this._data = e, this.isConnected && !this._rendering && this.render();
  }
  get data() {
    return j(this);
  }
  set format(e) {
    this._format = e || null, this.isConnected && !this._rendering && this.render();
  }
  get format() {
    return this._format || null;
  }
  set strings(e) {
    this._strings = e || null, this.isConnected && !this._rendering && this.render();
  }
  get strings() {
    return this._strings || null;
  }
  set locale(e) {
    this.setAttribute("locale", e ?? "");
  }
  get locale() {
    return this.getAttribute("locale");
  }
  set timeZone(e) {
    this.setAttribute("time-zone", e || "");
  }
  get timeZone() {
    return this.getAttribute("time-zone");
  }
  set theme(e) {
    e ? this.setAttribute("theme", e) : this.removeAttribute("theme");
  }
  get theme() {
    return this.getAttribute("theme");
  }
  set now(e) {
    this.setAttribute("now", e || "");
  }
  get now() {
    return this.getAttribute("now");
  }
  render() {
    if (this._rendering) return;
    this._rendering = !0;
    const e = this.shadowRoot;
    for (const r of [...e.childNodes])
      r.nodeName !== "STYLE" && e.removeChild(r);
    try {
      this.paint(e);
    } finally {
      this._rendering = !1;
    }
  }
  paint() {
  }
}
function Y(t, e, r) {
  t.dispatchEvent(new CustomEvent(e, { detail: r, bubbles: !0, composed: !0 }));
}
const ie = `
:host {
  color: var(--tl-text, #171a20);
  font-family: var(--tl-font, -apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif);
  font-size: 1rem;
  line-height: 1.5;
  box-sizing: border-box;
}
:host([theme="light"]) {
  color-scheme: light;
  --tl-bg: #f4f4f4; --tl-surface: #fff; --tl-surface-2: #f4f4f4; --tl-surface-3: #e9e9eb;
  --tl-line: rgba(23, 26, 32, 0.08); --tl-line-strong: rgba(23, 26, 32, 0.16);
  --tl-text: #171a20; --tl-muted: #5c5e62; --tl-faint: #8e9095;
  --tl-accent: #3e6ae1; --tl-accent-strong: #2f56c6; --tl-accent-soft: rgba(62, 106, 225, 0.1); --tl-on-accent: #fff;
  --tl-green: #1d9b52; --tl-green-soft: rgba(29, 155, 82, 0.12);
  --tl-amber: #c27c0e; --tl-violet: #7258e8; --tl-slate: #6b7280;
  --tl-danger: #c8332f; --tl-danger-soft: rgba(200, 51, 47, 0.1);
  --tl-map-bg: #eef0f3; --tl-map-dot: rgba(23, 26, 32, 0.12); --tl-map-road: rgba(23, 26, 32, 0.16);
  --tl-shadow: 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04);
}
:host([theme="dark"]) {
  color-scheme: dark;
  --tl-bg: #111318; --tl-surface: #181b21; --tl-surface-2: #20242b; --tl-surface-3: #2a2e36;
  --tl-line: rgba(255, 255, 255, 0.07); --tl-line-strong: rgba(255, 255, 255, 0.14);
  --tl-text: #f2f3f5; --tl-muted: #a3a6ad; --tl-faint: #7b7f87;
  --tl-accent: #6f93ff; --tl-accent-strong: #8fabff; --tl-accent-soft: rgba(111, 147, 255, 0.14); --tl-on-accent: #0b1430;
  --tl-green: #4fd17c; --tl-green-soft: rgba(79, 209, 124, 0.14);
  --tl-amber: #f2b441; --tl-violet: #a892ff; --tl-slate: #9aa1ad;
  --tl-danger: #ff7a70; --tl-danger-soft: rgba(255, 122, 112, 0.12);
  --tl-map-bg: #1d2128; --tl-map-dot: rgba(255, 255, 255, 0.08); --tl-map-road: rgba(255, 255, 255, 0.14);
  --tl-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
}
:host *, :host *::before, :host *::after { box-sizing: border-box; }
:focus { outline: none; }
:focus-visible { outline: 2px solid var(--tl-accent, #3e6ae1); outline-offset: 2px; border-radius: 6px; }
.visually-hidden { position: absolute !important; clip: rect(0 0 0 0); clip-path: inset(50%); height: 1px; overflow: hidden; white-space: nowrap; width: 1px; }
.muted { color: var(--tl-muted, #5c5e62); }
.figure, time { font-variant-numeric: tabular-nums; }
h2, h3 { font-weight: 500; letter-spacing: -0.01em; line-height: 1.2; margin: 0; }
h2 { font-size: 1.125rem; }
a { color: var(--tl-accent, #3e6ae1); }
.card-head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 8px 16px; margin-bottom: 12px; }
.card-head h2 { margin: 0; }
.text-link { color: var(--tl-accent, #3e6ae1); font-size: 0.875rem; font-weight: 500; text-decoration: none; }
.text-link:hover { text-decoration: underline; }
@keyframes tl-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
@keyframes tl-ripple { 0% { transform: scale(0.5); opacity: 0.6; } 100% { transform: scale(1.6); opacity: 0; } }
@keyframes tl-draw { from { stroke-dashoffset: 1; } to { stroke-dashoffset: 0; } }
@media (prefers-reduced-motion: reduce) {
  :host, :host *, :host *::before, :host *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
`, ae = `
:host {
  display: block;
  position: relative;
  width: 100%;
  min-width: 0;
  margin: 0;
  padding: 24px;
  border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08));
  border-radius: var(--tl-radius, 14px);
  background: var(--tl-surface, #fff);
  box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04));
}
.eyebrow { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin: 0; color: var(--tl-muted, #5c5e62); font-size: 0.75rem; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; }
.hero-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
.hero-name { margin: 6px 0 0; font-size: clamp(1.5rem, 5vw, 2rem); font-weight: 500; letter-spacing: -0.02em; }
.hero-car { margin: 16px -12px 8px; }
.hero-energy { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; margin: 0 0 12px; }
.hero-energy p { display: flex; flex-direction: column; margin: 0; }
.hero-range { text-align: right; align-items: flex-end; }
.hero-figure { font-size: clamp(2rem, 7vw, 2.75rem); font-weight: 400; letter-spacing: -0.03em; line-height: 1.05; font-variant-numeric: tabular-nums; }
.hero-label { color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.hero-note { margin: 12px 0 0; color: var(--tl-muted, #5c5e62); font-size: 0.9375rem; }
dl.hero-facts { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px 16px; margin: 20px 0 0; padding-top: 16px; border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
dt { color: var(--tl-muted, #5c5e62); font-size: 0.75rem; font-weight: 500; }
dd { margin: 2px 0 0; overflow-wrap: anywhere; }
.chip { display: inline-flex; align-items: center; gap: 6px; min-height: 28px; padding: 0 12px; border-radius: 999px; background: var(--tl-surface-2, #f4f4f4); color: var(--tl-text, #171a20); font-size: 0.8125rem; font-weight: 500; line-height: 1.2; white-space: nowrap; }
.status-dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; flex: none; }
.status.drive { background: var(--tl-accent-soft, rgba(62, 106, 225, 0.1)); color: var(--tl-accent, #3e6ae1); }
.status.charge { background: var(--tl-green-soft, rgba(29, 155, 82, 0.12)); color: var(--tl-green, #1d9b52); }
.status.park { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-text, #171a20); }
.status.note { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-amber, #c27c0e); }
.status.idle { background: var(--tl-surface-2, #f4f4f4); color: var(--tl-muted, #5c5e62); }
.status.drive .status-dot, .status.charge .status-dot { animation: tl-pulse 2s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
`, ne = `
:host { display: block; width: 100%; }
.stage { display: grid; aspect-ratio: 2 / 1; overflow: hidden; border-radius: var(--tl-radius-sm, 10px); background: var(--tl-surface-2, #f4f4f4); }
.stage > * { grid-area: 1 / 1; }
img { display: none; width: 100%; height: 100%; object-fit: cover; scale: 1.14;
  -webkit-mask-image: radial-gradient(ellipse 47% 60% at 50% 54%, #000 62%, transparent 100%);
  mask-image: radial-gradient(ellipse 47% 60% at 50% 54%, #000 62%, transparent 100%); }
:host(.ready) img { display: block; }
:host(.ready) .placeholder { display: none; }
.placeholder { display: flex; align-items: center; justify-content: center; margin: 0; padding: 16px; color: var(--tl-faint, #8e9095); font-size: 0.875rem; line-height: 1.4; text-align: center; }
`, se = `
:host { display: block; width: 100%; }
svg.battery-meter { display: block; width: 100%; height: 8px; overflow: visible; }
.battery-track { fill: var(--tl-surface-3, #e9e9eb); }
.battery-fill { fill: var(--tl-text, #171a20); }
.battery-meter.charging .battery-fill { fill: var(--tl-green, #1d9b52); }
.battery-meter.low .battery-fill { fill: var(--tl-danger, #c8332f); }
.battery-limit { fill: var(--tl-muted, #5c5e62); }
`, oe = `
:host { display: block; width: 100%; }
.map-stack { position: relative; overflow: hidden; background: var(--tl-map-bg, #eef0f3); }
.tiles { position: absolute; inset: 0; }
.tiles ::slotted(*) { width: 100%; height: 100%; object-fit: cover; }
svg.mini-map { position: relative; display: block; width: 100%; height: auto; aspect-ratio: 360 / 220; }
.map-bg { fill: var(--tl-map-bg, #eef0f3); }
.map-dot { fill: var(--tl-map-dot, rgba(23, 26, 32, 0.12)); }
.map-vignette { fill: var(--tl-map-bg, #eef0f3); opacity: 0.9; }
.map-road { fill: none; stroke: var(--tl-map-road, rgba(23, 26, 32, 0.16)); stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; }
.map-halo { fill: var(--tl-text, #171a20); opacity: 0.1; }
.mini-map.live .map-halo { fill: var(--tl-accent, #3e6ae1); opacity: 0.18; }
.mini-map.empty .map-halo { fill: var(--tl-faint, #8e9095); }
.map-heading { fill: var(--tl-text, #171a20); opacity: 0.75; }
.mini-map.live .map-heading { fill: var(--tl-accent, #3e6ae1); opacity: 1; }
.map-car { fill: var(--tl-text, #171a20); stroke: var(--tl-surface, #fff); stroke-width: 3; }
.mini-map.live .map-car { fill: var(--tl-accent, #3e6ae1); }
.map-pulse { fill: none; stroke: var(--tl-text, #171a20); stroke-width: 1.5; opacity: 0; transform-box: fill-box; transform-origin: center; }
.mini-map.live .map-pulse { stroke: var(--tl-accent, #3e6ae1); animation: tl-ripple 2.4s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
.map-scale path { fill: none; stroke: var(--tl-muted, #5c5e62); stroke-width: 1.5; }
.map-scale text { fill: var(--tl-muted, #5c5e62); font-family: var(--tl-font, sans-serif); font-size: 11px; dominant-baseline: middle; }
:host(.has-tiles) .map-bg, :host(.has-tiles) .map-dots { display: none; }
`, kt = `
:host { display: block; background: transparent; }
:host(:not(:first-child)) { border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
a.trip, .trip {
  display: grid;
  grid-template-columns: 3.25rem minmax(0, 1fr) auto;
  grid-template-areas: "times route figures" "times stats stats";
  gap: 6px 14px; align-items: start; width: 100%; padding: 14px 16px;
  color: inherit; text-decoration: none; box-sizing: border-box;
}
a.trip:hover, .trip:hover { background: var(--tl-surface-2, #f4f4f4); }
a.trip:focus-visible { outline-offset: -2px; border-radius: 0; }
.trip-times { grid-area: times; display: flex; flex-direction: column; gap: 2px; font-size: 0.875rem; }
.trip-times time { font-weight: 600; }
.trip-times > span { color: var(--tl-faint, #8e9095); }
.live { display: inline-flex; align-items: center; gap: 6px; color: var(--tl-green, #1d9b52); font-weight: 500; font-size: 0.75rem; }
.live-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--tl-green, #1d9b52); animation: tl-pulse 2s cubic-bezier(0.2, 0.7, 0.2, 1) infinite; }
.route { grid-area: route; position: relative; display: flex; flex-direction: column; gap: 4px; min-width: 0; }
.route::before { content: ""; position: absolute; left: 3.25px; top: 0.65em; bottom: 0.65em; width: 1.5px; background: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); }
.route.single::before { display: none; }
.route-stop { position: relative; display: block; padding-left: 18px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.route.single .route-stop { padding-left: 0; }
.route-stop::before { content: ""; position: absolute; left: 0; top: 0.5em; width: 8px; height: 8px; border-radius: 50%; box-sizing: border-box; }
.route-stop.start::before { border: 2px solid var(--tl-muted, #5c5e62); background: var(--tl-surface, #fff); }
.route-stop.end::before { background: var(--tl-accent, #3e6ae1); }
.route .charge-stop::before { display: none; }
.kind { display: flex; align-items: center; gap: 4px; font-size: 0.8125rem; color: var(--tl-muted, #5c5e62); }
.trip-figures { grid-area: figures; display: flex; flex-direction: column; align-items: flex-end; gap: 2px; text-align: right; font-size: 0.875rem; }
.trip-figures .figure { font-size: 1.0625rem; font-weight: 600; color: var(--tl-text, #171a20); white-space: nowrap; }
.trip.charge .trip-figures .figure { color: var(--tl-green, #1d9b52); }
.trip-stats { grid-area: stats; display: flex; flex-wrap: wrap; gap: 4px 18px; color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.trip-stats > span { display: flex; flex-direction: column; }
.stat-label { display: none; }
.chip.source { grid-column: 2 / -1; justify-self: start; min-height: 22px; font-size: 0.75rem; color: var(--tl-muted, #5c5e62); }
.chip.dc { min-height: 20px; padding: 0 7px; background: var(--tl-green-soft, rgba(29, 155, 82, 0.12)); color: var(--tl-green, #1d9b52); font-size: 0.6875rem; font-weight: 700; letter-spacing: 0.04em; }
svg.soc-bar { display: block; width: 100%; max-width: 9rem; height: 6px; margin-top: 6px; }
.soc-track { fill: var(--tl-surface-3, #e9e9eb); }
.soc-fill { fill: var(--tl-text, #171a20); }
.soc-bar.gain .soc-fill { fill: var(--tl-green, #1d9b52); }
@media (max-width: 640px) {
  a.trip, .trip { grid-template-columns: 2.75rem minmax(0, 1fr) auto; padding: 12px 14px; gap: 4px 12px; }
}
@media (min-width: 900px) {
  a.trip, .trip {
    grid-template-columns: 3.5rem minmax(0, 1.4fr) minmax(0, 1.6fr) 6.5rem;
    grid-template-areas: "times route stats figures";
    align-items: center;
  }
  .trip-stats { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; color: var(--tl-text, #171a20); font-size: 0.875rem; }
  .stat-label { display: block; color: var(--tl-faint, #8e9095); font-size: 0.6875rem; letter-spacing: 0.02em; }
  .chip.source { grid-column: 2; grid-row: 2; }
}
`, le = `
:host { display: block; margin: 0 0 24px; }
.day-head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 4px 12px; padding: 0 4px 8px; }
.day-head h2 { margin: 0; font-size: 1rem; font-weight: 600; }
.day-head p { margin: 0; font-size: 0.8125rem; }
.trips { overflow: hidden; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
`, ce = `
:host { display: block; padding: 24px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
.recent-link { display: block; margin: 0 -12px -12px; padding: 12px; border-radius: 10px; color: inherit; text-decoration: none; }
.recent-link:hover { background: var(--tl-surface-2, #f4f4f4); }
.recent-when { margin: 0 0 10px; color: var(--tl-muted, #5c5e62); font-size: 0.875rem; }
.recent-metrics { display: flex; align-items: baseline; gap: 12px; margin: 12px 0 0; }
.recent-metrics .figure { font-size: 1.375rem; font-weight: 500; letter-spacing: -0.02em; }
.route { position: relative; display: flex; flex-direction: column; gap: 4px; min-width: 0; margin-left: 2px; }
.route::before { content: ""; position: absolute; left: 3.25px; top: 0.65em; bottom: 0.65em; width: 1.5px; background: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); }
.route.single::before { display: none; }
.route-stop { position: relative; display: block; padding-left: 18px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.route.single .route-stop { padding-left: 0; }
.route-stop::before { content: ""; position: absolute; left: 0; top: 0.5em; width: 8px; height: 8px; border-radius: 50%; }
.route-stop.start::before { border: 2px solid var(--tl-muted, #5c5e62); background: var(--tl-surface, #fff); }
.route-stop.end::before { background: var(--tl-accent, #3e6ae1); }
.live-dot { display: inline-block; width: 7px; height: 7px; border-radius: 50%; background: var(--tl-green, #1d9b52); }
`, ue = `
:host { display: block; }
:host([variant="strip"]) { margin: 0 0 24px; }
:host([variant="card"]) { padding: 24px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
dl { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap: 24px; margin: 0; }
dl > div { padding: 12px 0; border-top: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); }
dd { margin-top: 4px; font-size: 1.125rem; font-weight: 500; letter-spacing: -0.01em; font-variant-numeric: tabular-nums; }
:host([variant="strip"]) dl { padding: 4px 20px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: var(--tl-radius, 14px); background: var(--tl-surface, #fff); box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04)); }
:host([variant="strip"]) dl > div { border-top: 0; }
@media (min-width: 720px) {
  :host([variant="strip"]) dl { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}
@media (min-width: 900px) {
  :host([variant="card"]) dl { grid-template-columns: repeat(4, minmax(0, 1fr)); }
}
`, de = `
:host { display: flex; flex-wrap: nowrap; gap: 2px; overflow-x: auto; scrollbar-width: none; padding: 3px; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); width: max-content; max-width: 100%; }
:host::-webkit-scrollbar { display: none; }
a { display: inline-flex; align-items: center; min-height: 32px; padding: 0 14px; border-radius: 999px; color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; font-weight: 500; text-decoration: none; white-space: nowrap; }
a:hover { color: var(--tl-text, #171a20); }
a[aria-current="page"] { background: var(--tl-surface, #fff); color: var(--tl-text, #171a20); box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12); }
`, he = `
:host { display: flex; flex-wrap: nowrap; gap: 4px; overflow-x: auto; scrollbar-width: none; margin: 0 0 24px; padding: 4px; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); width: max-content; max-width: 100%; }
:host::-webkit-scrollbar { display: none; }
a { display: inline-flex; align-items: center; justify-content: center; min-height: 36px; padding: 0 16px; border-radius: 999px; color: var(--tl-muted, #5c5e62); font-size: 0.875rem; font-weight: 500; text-decoration: none; white-space: nowrap; }
a:hover { color: var(--tl-text, #171a20); }
a[aria-selected="true"] { background: var(--tl-surface, #fff); color: var(--tl-text, #171a20); box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12); }
`, fe = `
:host { display: block; position: relative; color: var(--tl-text, #171a20); }
:host([variant="card"]) {
  padding: 20px;
  border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08));
  border-radius: var(--tl-radius, 14px);
  background: var(--tl-surface, #fff);
  box-shadow: var(--tl-shadow, 0 1px 2px rgba(23, 26, 32, 0.04), 0 8px 24px rgba(23, 26, 32, 0.04));
}
figure.chart { position: relative; margin: 0; padding: 0; }
figure.chart figcaption { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 2px 12px; margin: 0 0 8px; }
.chart-title { font-size: 0.9375rem; font-weight: 600; }
.chart-summary { color: var(--tl-muted, #5c5e62); font-size: 0.8125rem; }
.chart-plot { position: relative; min-height: 180px; touch-action: pan-y; user-select: none; }
svg.chart-svg { display: block; width: 100%; overflow: visible; }
.chart-grid-line { stroke: var(--tl-line, rgba(23, 26, 32, 0.08)); stroke-width: 1; shape-rendering: crispEdges; }
.chart-zero { stroke: var(--tl-line-strong, rgba(23, 26, 32, 0.16)); stroke-width: 1; shape-rendering: crispEdges; }
.chart-axis text, .chart-tick { fill: var(--tl-faint, #8e9095); font-family: var(--tl-font, sans-serif); font-size: 11px; font-variant-numeric: tabular-nums; }
.chart-line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; vector-effect: non-scaling-stroke; stroke: currentColor; }
.chart-area { stroke: none; }
.chart-bar { fill: currentColor; transition: opacity 160ms cubic-bezier(0.2, 0.7, 0.2, 1); }
.chart-bars:hover .chart-bar { opacity: 0.45; }
.chart-bars .chart-bar.on, .chart-bars:hover .chart-bar.on { opacity: 1; }
.chart-bar-hit { fill: transparent; cursor: pointer; }
.chart-crosshair { stroke: var(--tl-muted, #5c5e62); stroke-width: 1; stroke-dasharray: 3 3; opacity: 0; shape-rendering: crispEdges; }
.chart-dot { fill: currentColor; stroke: var(--tl-surface, #fff); stroke-width: 2; opacity: 0; }
.chart-on .chart-crosshair, .chart-on .chart-dot { opacity: 1; }
.chart-brush { fill: var(--tl-accent, #3e6ae1); opacity: 0.12; }
.tone-accent { color: var(--tl-accent, #3e6ae1); }
.tone-green { color: var(--tl-green, #1d9b52); }
.tone-amber { color: var(--tl-amber, #c27c0e); }
.tone-violet { color: var(--tl-violet, #7258e8); }
.tone-slate { color: var(--tl-slate, #6b7280); }
.chart-tooltip { position: absolute; top: 0; left: 0; z-index: 3; min-width: 8rem; padding: 8px 10px; border: 1px solid var(--tl-line, rgba(23, 26, 32, 0.08)); border-radius: 10px; background: var(--tl-surface, #fff); box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12); color: var(--tl-text, #171a20); font-size: 0.8125rem; line-height: 1.35; pointer-events: none; opacity: 0; white-space: nowrap; }
.chart-on .chart-tooltip { opacity: 1; }
.chart-tooltip .tip-head { margin-bottom: 4px; color: var(--tl-muted, #5c5e62); }
.chart-tooltip .tip-row { display: flex; align-items: center; gap: 6px; }
.chart-tooltip .tip-row b { margin-left: auto; padding-left: 12px; font-weight: 600; }
.chart-tooltip .tip-swatch, .chart-legend .tip-swatch { width: 8px; height: 8px; border-radius: 2px; background: currentColor; }
.chart-legend { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: -2px 0 8px; color: var(--tl-muted, #5c5e62); font-size: 0.75rem; }
.chart-legend > span { display: inline-flex; align-items: center; gap: 6px; }
.chart-tools { position: absolute; top: 8px; right: 0; display: flex; gap: 4px; opacity: 0; }
figure.chart.zoomed .chart-tools { opacity: 1; }
figure.chart:not(.chart-lead) .chart-tools { display: none; }
.chart-tools button { min-height: 28px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--tl-surface-3, #e9e9eb); color: var(--tl-text, #171a20); font: inherit; font-size: 0.75rem; cursor: pointer; }
.chart-ready .chart-fallback { display: none; }
svg.chart-fallback { display: block; width: 100%; height: 180px; color: var(--tl-accent, #3e6ae1); }
`;
function P(...t) {
  return ie + t.join(`
`);
}
var U, nt, Q;
class pe extends _ {
  constructor() {
    super(P(se));
    dt(this, U);
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "level", "limit", "charging", "low-at"];
  }
  set level(r) {
    R(this, U, nt).call(this, "level", r);
  }
  get level() {
    return R(this, U, Q).call(this, "level");
  }
  set limit(r) {
    R(this, U, nt).call(this, "limit", r);
  }
  get limit() {
    return R(this, U, Q).call(this, "limit");
  }
  set charging(r) {
    r ? this.setAttribute("charging", "") : this.removeAttribute("charging");
  }
  get charging() {
    return this.hasAttribute("charging");
  }
  paint(r) {
    const i = I(this), a = this.level;
    if (a == null) return;
    const n = Math.min(100, Math.max(0, a)), l = this.charging, o = R(this, U, Q).call(this, "low-at") ?? 20;
    let s = "battery-meter";
    l ? s += " charging" : n <= o && (s += " low");
    const c = x("svg", {
      class: s,
      viewBox: "0 0 100 8",
      preserveAspectRatio: "none",
      role: "img",
      "aria-label": `${i.strings.battery} ${Math.round(n)} %`,
      focusable: "false"
    }, r);
    x("rect", { class: "battery-track", width: "100", height: "8", rx: "4" }, c), x("rect", {
      class: "battery-fill",
      width: Math.max(2, n).toFixed(1),
      height: "8",
      rx: "4"
    }, c);
    const d = this.limit;
    d != null && d > 0 && d < 100 && x("rect", {
      class: "battery-limit",
      x: (d - 0.4).toFixed(1),
      y: "0",
      width: "0.8",
      height: "8"
    }, c);
  }
}
U = new WeakSet(), nt = function(r, i) {
  i == null || i === "" || Number.isNaN(Number(i)) ? this.removeAttribute(r) : this.setAttribute(r, String(i));
}, Q = function(r) {
  const i = this.getAttribute(r);
  if (i == null || i === "") return null;
  const a = Number(i);
  return Number.isFinite(a) ? a : null;
};
class At extends _ {
  constructor() {
    super(P(ne));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "image-url", "alt", "unavailable"];
  }
  set imageUrl(e) {
    e ? this.setAttribute("image-url", e) : this.removeAttribute("image-url");
  }
  get imageUrl() {
    return this.getAttribute("image-url");
  }
  paint(e) {
    const r = I(this), i = this.getAttribute("image-url") || "", a = this.getAttribute("unavailable") || r.strings.unavailableImage, n = this.getAttribute("alt") || a;
    this.classList.remove("ready");
    const l = u("div", { class: "stage" }, e), o = u("p", { class: "placeholder", text: a }, l);
    if (o.setAttribute("role", "img"), o.setAttribute("aria-label", a), !i) return;
    const s = u("img", { alt: n, decoding: "async" }, l), c = () => {
      s.naturalWidth > 0 && this.classList.add("ready");
    }, d = () => this.classList.remove("ready");
    s.addEventListener("load", c), s.addEventListener("error", d), s.src = i, s.complete && (s.naturalWidth > 0 ? c() : d());
  }
}
var ge = "http://www.w3.org/2000/svg", K = " ", bt = window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : null, H = {}, me = 0;
function be() {
  return bt && bt.matches;
}
function y(t, e, r) {
  var i = document.createElementNS(ge, t);
  return e && Object.keys(e).forEach(function(a) {
    i.setAttribute(a, e[a]);
  }), r && r.appendChild(i), i;
}
function D(t, e, r) {
  var i = document.createElement(t);
  return e && (i.className = e), r && r.appendChild(i), i;
}
function rt(t, e) {
  if (t == null || !isFinite(t)) return "—";
  var r = Number(t).toLocaleString("fr-FR", {
    minimumFractionDigits: e,
    maximumFractionDigits: e
  });
  return r.replace("-", "−");
}
function it(t, e, r) {
  var i = rt(t, e);
  return i === "—" || !r ? i : i + K + r;
}
function tt(t) {
  return t < 10 ? "0" + t : String(t);
}
function _t(t, e) {
  var r = t / Math.max(1, e), i = Math.pow(10, Math.floor(Math.log(r) / Math.LN10)), a = r / i, n = a < 1.5 ? 1 : a < 3 ? 2 : a < 7 ? 5 : 10;
  return n * i;
}
function Mt(t, e, r, i) {
  i && (t = Math.min(t, 0), e = Math.max(e, 0)), e > t || (e = t + 1, i || (t = t - 1));
  for (var a = _t(e - t, r), n = Math.floor(t / a) * a, l = Math.ceil(e / a) * a, o = [], s = n; s <= l + a / 2; s += a) o.push(Math.abs(s) < a / 1e6 ? 0 : s);
  return { lo: n, hi: l, ticks: o, step: a };
}
function Ct(t) {
  return t >= 1 ? 0 : Math.min(3, Math.ceil(-Math.log(t) / Math.LN10));
}
function Nt(t, e, r) {
  var i = new Date(t.origin_ms + e * 1e3 + t.offset_min * 6e4), a = tt(i.getUTCHours()) + ":" + tt(i.getUTCMinutes());
  return r ? a + ":" + tt(i.getUTCSeconds()) : a;
}
function ve(t) {
  var e = Math.round(t / 60);
  return e < 60 ? e + K + "min" : Math.floor(e / 60) + K + "h" + K + tt(e % 60);
}
function St(t, e, r) {
  return t.kind === "time" ? Nt(t, e, r) : it(e, t.decimals || 0, t.unit);
}
function xe(t, e, r, i) {
  for (var a = [60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400], n = r - e, l = a[a.length - 1], o = 0; o < a.length; o++)
    if (n / a[o] <= i) {
      l = a[o];
      break;
    }
  for (var s = t.origin_ms / 1e3 + t.offset_min * 60, c = Math.ceil((s + e) / l) * l - s, d = [], h = c; h <= r; h += l) d.push(h);
  return d;
}
function ye(t, e, r) {
  for (var i = _t(e - t, r), a = [], n = Math.ceil(t / i) * i; n <= e + i / 1e6; n += i) a.push(n);
  return a;
}
function J(t, e) {
  for (var r = 0, i = t.length - 1; i - r > 1; ) {
    var a = r + i >> 1;
    t[a] < e ? r = a : i = a;
  }
  return Math.abs(t[r] - e) <= Math.abs(t[i] - e) ? r : i;
}
function we(t) {
  return 1 - Math.pow(1 - t, 3);
}
function $t(t, e, r) {
  if (be()) {
    e(1);
    return;
  }
  var i = null;
  function a(n) {
    i === null && (i = n);
    var l = Math.min(1, (n - i) / t);
    e(we(l)), l < 1 && window.requestAnimationFrame(a);
  }
  window.requestAnimationFrame(a);
}
function N(t, e) {
  if (this.figure = t, this.data = e, this.id = "c" + ++me, this.plot = t.querySelector(".chart-plot"), this.sync = t.getAttribute("data-sync"), this.active = -1, this.reveal = 0, e.kind === "line" && (this.full = [e.t[0], e.t[e.t.length - 1]], this.view = this.full.slice()), (!this.sync || !H[this.sync]) && t.classList.add("chart-lead"), this.build(), this.sync && ((H[this.sync] = H[this.sync] || []).push(this), this.map = document.querySelector('svg.map[data-sync="' + this.sync + '"]'), this.map)) {
    try {
      this.mapXY = JSON.parse(this.map.getAttribute("data-xy") || "[]");
    } catch {
      this.mapXY = [];
    }
    this.mapCursor = this.map.querySelector(".map-cursor");
  }
}
N.prototype.build = function() {
  var t = this;
  if (this.figure.classList.add("chart-ready"), this.root = y("svg", { class: "chart-svg", role: "presentation", "aria-hidden": "true", focusable: "false" }, this.plot), this.tip = D("div", "chart-tooltip", this.plot), this.tip.setAttribute("aria-hidden", "true"), this.live = D("p", "visually-hidden", this.figure), this.live.setAttribute("aria-live", "polite"), this.data.kind === "line") {
    var e = D("div", "chart-tools", this.figure), r = D("button", "ghost small", e);
    r.type = "button", r.textContent = "Réinitialiser le zoom", r.addEventListener("click", function() {
      t.zoomTo(t.full[0], t.full[1], !0);
    });
  }
  if (this.data.series.length > 1) {
    var i = D("p", "chart-legend", this.figure);
    this.data.series.forEach(function(a) {
      var n = D("span", "", i);
      D("span", "tip-swatch tone-" + a.tone, n), n.appendChild(document.createTextNode(a.name));
    }), this.figure.insertBefore(i, this.plot);
  }
  this.bind(), this.render(), $t(700, function(a) {
    t.reveal = a, t.render();
  });
};
N.prototype.size = function() {
  var t = Math.max(240, Math.round(this.plot.clientWidth || 600)), e = t < 520 ? 170 : 200;
  return this.data.kind === "bar" && (e += 16), { width: t, height: e, left: 44, right: 10, top: 10, bottom: 24 };
};
N.prototype.render = function() {
  this.data.kind === "bar" ? this.renderBars() : this.renderLines();
};
N.prototype.renderLines = function() {
  for (var t = this.data, e = this.size(), r = this.root; r.firstChild; ) r.removeChild(r.firstChild);
  r.setAttribute("viewBox", "0 0 " + e.width + " " + e.height), r.setAttribute("width", e.width), r.setAttribute("height", e.height);
  var i = e.width - e.left - e.right, a = e.height - e.top - e.bottom, n = this.view[0], l = this.view[1] > n ? this.view[1] : n + 1, o = t.t, s = Math.max(0, J(o, n) - 1), c = Math.min(o.length - 1, J(o, l) + 1), d = 1 / 0, h = -1 / 0, g = !1;
  t.series.forEach(function(p) {
    p.signed && (g = !0);
    for (var T = s; T <= c; T++) {
      var S = p.values[T];
      S != null && (S < d && (d = S), S > h && (h = S));
    }
  }), isFinite(d) || (d = 0, h = 1);
  var f = Mt(d, h, e.height < 190 ? 3 : 4, g), A = Ct(f.step);
  function b(p) {
    return e.left + (p - n) / (l - n) * i;
  }
  function w(p) {
    return e.top + a - (p - f.lo) / (f.hi - f.lo) * a;
  }
  this.px = b, this.py = w, this.box = e;
  var k = y("g", { class: "chart-axis" }, r);
  f.ticks.forEach(function(p) {
    var T = Math.round(w(p)) + 0.5;
    y("line", { class: p === 0 && g ? "chart-zero" : "chart-grid-line", x1: e.left, x2: e.width - e.right, y1: T, y2: T }, k);
    var S = y("text", { class: "chart-tick", x: e.left - 8, y: T, "text-anchor": "end", "dominant-baseline": "middle" }, k);
    S.textContent = rt(p, A);
  });
  var C = t.x.kind === "time" ? xe(t.x, n, l, Math.floor(i / 72)) : ye(n, l, Math.floor(i / 72));
  C.forEach(function(p) {
    var T = y("text", { class: "chart-tick", x: b(p), y: e.height - 6, "text-anchor": "middle" }, k);
    T.textContent = t.x.kind === "time" ? Nt(t.x, p, !1) : rt(p, t.x.decimals || 0);
  });
  var E = y("defs", null, r), v = y("clipPath", { id: this.id + "-clip" }, E);
  y("rect", { x: e.left, y: 0, width: Math.max(0, i * this.reveal), height: e.height }, v);
  var m = y("g", { "clip-path": "url(#" + this.id + "-clip)" }, r), M = this;
  t.series.forEach(function(p, T) {
    for (var S = [], B = [], W = s; W <= c; W++) {
      var $ = p.values[W];
      if ($ == null) {
        B.length && S.push(B), B = [];
        continue;
      }
      B.push([b(o[W]), w($)]);
    }
    B.length && S.push(B);
    var z = "", F = "", O = g ? w(0) : e.top + a;
    S.forEach(function(X) {
      var lt = X.map(function(ct, Dt) {
        return (Dt ? "L" : "M") + ct[0].toFixed(1) + " " + ct[1].toFixed(1);
      }).join("");
      z += lt, p.area && X.length > 1 && (F += lt + "L" + X[X.length - 1][0].toFixed(1) + " " + O.toFixed(1) + "L" + X[0][0].toFixed(1) + " " + O.toFixed(1) + "Z");
    });
    var q = "tone-" + p.tone;
    if (F) {
      var V = M.id + "-g" + T, Z = y("linearGradient", { id: V, class: q, x1: 0, y1: 0, x2: 0, y2: 1 }, E);
      y("stop", { offset: 0, "stop-color": "currentColor", "stop-opacity": g ? 0.22 : 0.26 }, Z), y("stop", { offset: 1, "stop-color": "currentColor", "stop-opacity": 0.02 }, Z), y("path", { class: "chart-area", d: F, fill: "url(#" + V + ")" }, m);
    }
    y("path", { class: "chart-line " + q, d: z }, m);
  });
  var L = y("g", { class: "chart-cursor" }, r);
  this.crosshair = y("line", { class: "chart-crosshair", x1: -10, x2: -10, y1: e.top, y2: e.top + a }, L), this.dots = t.series.map(function(p) {
    return y("circle", { class: "chart-dot tone-" + p.tone, r: 4, cx: -10, cy: -10 }, L);
  }), this.brush = y("rect", { class: "chart-brush", x: 0, y: e.top, width: 0, height: a }, r), this.figure.classList.toggle("zoomed", this.view[0] > this.full[0] || this.view[1] < this.full[1]), this.active >= 0 && this.showIndex(this.active, !1);
};
N.prototype.renderBars = function() {
  var t = this.data, e = this.size();
  e.bottom = 30;
  for (var r = this.root; r.firstChild; ) r.removeChild(r.firstChild);
  r.setAttribute("viewBox", "0 0 " + e.width + " " + e.height), r.setAttribute("width", e.width), r.setAttribute("height", e.height);
  for (var i = e.width - e.left - e.right, a = e.height - e.top - e.bottom, n = t.labels.length, l = [], o = 0, s = 0; s < n; s++) {
    var c = 0, d = 0;
    t.series.forEach(function($) {
      var z = $.values[s] || 0;
      c += z, z > d && (d = z);
    }), l.push(c), o = Math.max(o, t.stacked ? c : d);
  }
  var h = Mt(0, o || 1, e.height < 200 ? 3 : 4, !1), g = Ct(h.step);
  function f($) {
    return e.top + a - ($ - h.lo) / (h.hi - h.lo) * a;
  }
  var A = y("g", { class: "chart-axis" }, r);
  h.ticks.forEach(function($) {
    var z = Math.round(f($)) + 0.5;
    y("line", { class: "chart-grid-line", x1: e.left, x2: e.width - e.right, y1: z, y2: z }, A);
    var F = y("text", { class: "chart-tick", x: e.left - 8, y: z, "text-anchor": "end", "dominant-baseline": "middle" }, A);
    F.textContent = rt($, g);
  });
  var b = i / Math.max(1, n), w = Math.max(3, Math.min(42, b * 0.62)), k = Math.max(1, Math.ceil(52 / b)), C = y("g", { class: "chart-bars" }, r), E = this;
  this.barGroups = [];
  for (var v = 0; v < n; v++) {
    var m = e.left + b * v + b / 2, M = y("g", { class: "chart-bar-group" }, C), L = f(0), p = t.series;
    if (t.stacked) {
      var T = 0;
      p.forEach(function($, z) {
        var F = ($.values[v] || 0) * E.reveal;
        if (!(F <= 0)) {
          var O = f(T), q = f(T + F);
          T += F;
          var V = z === p.length - 1 || p.slice(z + 1).every(function(Z) {
            return !(Z.values[v] > 0);
          });
          y("rect", { class: "chart-bar tone-" + $.tone, x: (m - w / 2).toFixed(1), y: q.toFixed(1), width: w.toFixed(1), height: Math.max(0.5, O - q).toFixed(1), rx: V ? Math.min(4, w / 4) : 0 }, M);
        }
      });
    } else {
      var S = w / p.length;
      p.forEach(function($, z) {
        var F = $.values[v];
        if (F != null) {
          var O = f(F * E.reveal);
          y("rect", { class: "chart-bar tone-" + $.tone, x: (m - w / 2 + S * z).toFixed(1), y: Math.min(O, L).toFixed(1), width: Math.max(1, S - 1).toFixed(1), height: Math.max(0.5, Math.abs(L - O)).toFixed(1), rx: Math.min(4, S / 4) }, M);
        }
      });
    }
    var B = y("rect", { class: "chart-bar-hit", x: e.left + b * v, y: e.top, width: b, height: a + e.bottom }, M);
    if (B.setAttribute("data-index", v), this.barGroups.push(M), v % k === 0) {
      var W = y("text", { class: "chart-tick", x: m, y: e.height - 10, "text-anchor": "middle" }, A);
      W.textContent = t.labels[v];
    }
  }
  this.barBand = b, this.box = e, this.totals = l, this.active >= 0 && this.showBar(this.active, !1);
};
N.prototype.bind = function() {
  var t = this, e = this.plot, r = null;
  function i(o) {
    var s = e.getBoundingClientRect(), c = t.box ? t.box.width / s.width : 1;
    return (o.clientX - s.left) * c;
  }
  function a(o) {
    var s = t.box, c = (o - s.left) / (s.width - s.left - s.right);
    return t.view[0] + Math.max(0, Math.min(1, c)) * (t.view[1] - t.view[0]);
  }
  e.addEventListener("pointermove", function(o) {
    var s = i(o);
    if (t.data.kind === "bar") {
      var c = Math.floor((s - t.box.left) / t.barBand);
      c >= 0 && c < t.data.labels.length && t.showBar(c, !0);
      return;
    }
    if (r) {
      r.current = s;
      var d = Math.max(t.box.left, Math.min(r.start, s)), h = Math.min(t.box.width - t.box.right, Math.max(r.start, s));
      t.brush.setAttribute("x", d), t.brush.setAttribute("width", Math.max(0, h - d));
    }
    var g = J(t.data.t, a(s));
    t.broadcast(g);
  }), e.addEventListener("pointerleave", function() {
    r || t.clearGroup();
  }), e.addEventListener("pointerdown", function(o) {
    t.data.kind !== "line" || o.button !== 0 || (r = { start: i(o), current: i(o), pointer: o.pointerId }, e.setPointerCapture && e.setPointerCapture(o.pointerId));
  });
  function n(o) {
    if (r) {
      var s = r.start, c = r.current;
      if (e.releasePointerCapture && r.pointer !== void 0)
        try {
          e.releasePointerCapture(r.pointer);
        } catch {
        }
      if (r = null, t.brush.setAttribute("width", 0), Math.abs(c - s) > 12) {
        var d = a(Math.min(s, c)), h = a(Math.max(s, c));
        t.zoomTo(d, h, !0);
      }
      o && o.type === "pointercancel" && t.clearGroup();
    }
  }
  if (e.addEventListener("pointerup", n), e.addEventListener("pointercancel", n), e.addEventListener("dblclick", function() {
    t.data.kind === "line" && t.zoomTo(t.full[0], t.full[1], !0);
  }), e.addEventListener("click", function(o) {
    if (!(t.data.kind !== "bar" || !t.data.hrefs)) {
      var s = o.target, c = s && s.getAttribute ? s.getAttribute("data-index") : null;
      c !== null && t.data.hrefs[c] && (window.location.href = t.data.hrefs[c]);
    }
  }), this.figure.addEventListener("keydown", function(o) {
    var s = t.data.kind === "bar" ? t.data.labels.length : t.data.t.length, c = o.shiftKey ? 10 : 1, d = t.active;
    d < 0 && (d = t.data.kind === "bar" ? s - 1 : J(t.data.t, (t.view[0] + t.view[1]) / 2));
    var h = !0;
    switch (o.key) {
      case "ArrowRight":
        d = Math.min(s - 1, d + c);
        break;
      case "ArrowLeft":
        d = Math.max(0, d - c);
        break;
      case "Home":
        d = 0;
        break;
      case "End":
        d = s - 1;
        break;
      case "+":
      case "=":
        if (t.data.kind !== "line") return;
        t.zoomAround(t.data.t[d], 0.5);
        break;
      case "-":
      case "_":
        if (t.data.kind !== "line") return;
        t.zoomAround(t.data.t[d], 2);
        break;
      case "0":
      case "Escape":
        t.data.kind === "line" && t.zoomTo(t.full[0], t.full[1], !0), t.clearGroup();
        return;
      case "Enter":
        t.data.kind === "bar" && t.data.hrefs && t.data.hrefs[d] && (window.location.href = t.data.hrefs[d]);
        return;
      default:
        h = !1;
    }
    if (h) {
      if (o.preventDefault(), t.data.kind === "bar") t.showBar(d, !0);
      else {
        var g = t.data.t[d];
        if (g < t.view[0] || g > t.view[1]) {
          var f = t.view[1] - t.view[0], A = Math.max(t.full[0], Math.min(t.full[1] - f, g - f / 2));
          t.zoomTo(A, A + f, !0);
        }
        t.broadcast(d);
      }
      t.announce(d);
    }
  }), this.figure.addEventListener("blur", function() {
    t.clearGroup();
  }), window.ResizeObserver) {
    var l = 0;
    new ResizeObserver(function() {
      var o = Math.round(e.clientWidth);
      o !== l && (l = o, t.render());
    }).observe(e);
  } else
    window.addEventListener("resize", function() {
      t.render();
    });
};
N.prototype.zoomAround = function(t, e) {
  var r = (this.view[1] - this.view[0]) * e, i = this.full[1] - this.full[0];
  r = Math.max(Math.min(r, i), i / 200);
  var a = Math.max(this.full[0], Math.min(this.full[1] - r, t - r / 2));
  this.zoomTo(a, a + r, !0);
};
N.prototype.zoomTo = function(t, e, r) {
  if (this.data.kind === "line") {
    var i = (this.full[1] - this.full[0]) / 200;
    if (e - t < i) {
      var a = (t + e) / 2;
      t = a - i / 2, e = a + i / 2;
    }
    t = Math.max(this.full[0], t), e = Math.min(this.full[1], e);
    var n = this, l = this.view.slice();
    $t(320, function(o) {
      n.view = [l[0] + (t - l[0]) * o, l[1] + (e - l[1]) * o], n.render();
    }), r && this.sync && this.data.x.kind === "time" && (H[this.sync] || []).forEach(function(o) {
      o !== n && o.data.kind === "line" && o.data.x.kind === "time" && o.zoomTo(t, e, !1);
    });
  }
};
N.prototype.broadcast = function(t) {
  var e = this.data.t[t], r = this;
  this.showIndex(t, !0), this.sync && (H[this.sync] || []).forEach(function(i) {
    i !== r && i.data.kind === "line" && i.showIndex(J(i.data.t, e), !1);
  }), this.moveMap(t);
};
N.prototype.clearGroup = function() {
  var t = this.sync ? H[this.sync] || [this] : [this];
  t.forEach(function(e) {
    e.clear();
  }), this.mapCursor && this.mapCursor.classList.remove("on"), this.sync && window.teslaMapHide && window.teslaMapHide(this.sync);
};
N.prototype.clear = function() {
  this.active = -1, this.figure.classList.remove("chart-on"), this.barGroups && this.barGroups.forEach(function(t) {
    t.querySelectorAll(".chart-bar").forEach(function(e) {
      e.classList.remove("on");
    });
  });
};
N.prototype.moveMap = function(t) {
  if (this.mapCursor && this.mapXY && this.mapXY[t] && this.mapXY[t][0] >= -50) {
    var e = this.mapXY[t];
    this.mapCursor.setAttribute("cx", e[0]), this.mapCursor.setAttribute("cy", e[1]), this.mapCursor.classList.add("on");
  }
  this.sync && window.teslaMapMove && window.teslaMapMove(this.sync, t);
};
N.prototype.showIndex = function(t, e) {
  var r = this.data, i = r.t[t];
  if (!(i === void 0 || !this.px)) {
    this.active = t;
    var a = this.px(i), n = i >= this.view[0] && i <= this.view[1];
    this.crosshair.setAttribute("x1", a), this.crosshair.setAttribute("x2", a);
    var l = this, o = [];
    r.series.forEach(function(s, c) {
      var d = s.values[t], h = l.dots[c];
      if (d == null || !n) {
        h.setAttribute("cx", -10);
        return;
      }
      h.setAttribute("cx", a), h.setAttribute("cy", l.py(d)), o.push([s, d]);
    }), this.figure.classList.toggle("chart-on", n), this.fillTip(St(r.x, i, !0) + (r.x.kind === "time" && i > 0 ? " · +" + ve(i) : ""), o, a, e);
  }
};
N.prototype.showBar = function(t, e) {
  var r = this.data;
  this.active = t, this.barGroups.forEach(function(n, l) {
    n.querySelectorAll(".chart-bar").forEach(function(o) {
      o.classList.toggle("on", l === t);
    });
  });
  var i = r.series.map(function(n) {
    return [n, n.values[t]];
  });
  r.stacked && r.series.length > 1 && i.push([{ name: "Total", unit: r.series[0].unit, decimals: r.series[0].decimals, tone: "" }, this.totals[t]]), this.figure.classList.add("chart-on");
  var a = this.box.left + this.barBand * t + this.barBand / 2;
  this.fillTip(r.titles && r.titles[t] || r.labels[t], i, a, e);
};
N.prototype.fillTip = function(t, e, r, i) {
  for (var a = this.tip; a.firstChild; ) a.removeChild(a.firstChild);
  if (!i) {
    a.classList.add("visually-hidden");
    return;
  }
  a.classList.remove("visually-hidden"), D("div", "tip-head", a).textContent = t, e.forEach(function(c) {
    var d = D("div", "tip-row", a);
    c[0].tone && D("span", "tip-swatch tone-" + c[0].tone, d);
    var h = D("span", "", d);
    h.textContent = c[0].name;
    var g = D("b", "", d);
    g.textContent = it(c[1], c[0].decimals, c[0].unit);
  });
  var n = this.plot.clientWidth || 1, l = n / this.box.width, o = r * l + 14, s = a.offsetWidth || 140;
  o + s > n && (o = r * l - s - 14), a.style.transform = "translate(" + Math.max(0, Math.round(o)) + "px, 4px)";
};
N.prototype.announce = function(t) {
  var e = this.data, r, i = [];
  e.kind === "bar" ? (r = e.titles && e.titles[t] || e.labels[t], e.series.forEach(function(a) {
    i.push(a.name + " " + it(a.values[t], a.decimals, a.unit));
  })) : (r = St(e.x, e.t[t], !1), e.series.forEach(function(a) {
    i.push(a.name + " " + it(a.values[t], a.decimals, a.unit));
  })), this.live.textContent = r + " : " + i.join(", ");
};
function Oe(t) {
  var e = t || document, r = e.querySelectorAll("figure.chart[data-chart]");
  Array.prototype.forEach.call(r, function(i) {
    if (!(i.closest && i.closest("tl-chart"))) {
      var a;
      try {
        a = JSON.parse(i.getAttribute("data-chart"));
      } catch {
        return;
      }
      Et(i, a);
    }
  });
}
function Et(t, e) {
  return !t || !e || !e.series || !e.series.length || e.kind === "line" && (!e.t || e.t.length < 2) || e.kind === "bar" && (!e.labels || !e.labels.length) ? null : new N(t, e);
}
function ke(t) {
  var e;
  if (!t || t.kind === "line") {
    const r = [], i = (e = t == null ? void 0 : t.series) == null ? void 0 : e[0], a = (t == null ? void 0 : t.t) || [];
    if (i && a.length > 1 && a.forEach((f, A) => {
      const b = i.values[A];
      b != null && r.push([f, b]);
    }), r.length < 2) return null;
    const n = r[0][0], l = r[r.length - 1][0] || 1, o = Math.min(...r.map((f) => f[1])), s = Math.max(...r.map((f) => f[1])), c = l - n || 1, d = s - o || 1, h = r.map(([f, A]) => {
      const b = (f - n) / c * 100, w = 40 - (A - o) / d * 36;
      return `${b.toFixed(2)},${w.toFixed(2)}`;
    }).join(" "), g = x("svg", {
      class: "chart-fallback",
      viewBox: "0 0 100 44",
      width: "640",
      height: "180",
      preserveAspectRatio: "none",
      role: "img",
      "aria-hidden": "true"
    });
    return x("polyline", { fill: "none", stroke: "currentColor", "stroke-width": "1.2", points: h }, g), g;
  }
  return null;
}
class Ae extends _ {
  constructor() {
    super(P(fe));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "title", "summary", "variant", "sync"];
  }
  paint(e) {
    const r = this._chart(), i = this.getAttribute("title") || (r == null ? void 0 : r.title) || "", a = this.getAttribute("summary") || (r == null ? void 0 : r.summary) || "", n = u("figure", { class: "chart", tabindex: "0", "aria-label": `${i}. ${a}` }, e);
    this.getAttribute("sync") && n.setAttribute("data-sync", this.getAttribute("sync"));
    const l = u("figcaption", null, n);
    u("span", { class: "chart-title", text: i }, l), u("span", { class: "chart-summary", text: a }, l);
    const o = u("div", { class: "chart-plot" }, n), s = ke(r);
    s && o.append(s), r && (Et(n, r), this.classList.add("ready"));
  }
  _chart() {
    const e = j(this);
    if (e && e.series) return e;
    const r = this.querySelector(":scope > figure.chart"), i = r == null ? void 0 : r.getAttribute("data-chart");
    if (!i) return e;
    try {
      return JSON.parse(i);
    } catch {
      return null;
    }
  }
}
function _e(t, e) {
  const r = st(Number(t.energy_used_kwh), Number(t.distance_km));
  return [
    { label: e.strings.drives, value: e.number(Number(t.drive_count) || 0, 0) },
    { label: e.strings.distance, value: e.distance(t.distance_km) },
    { label: e.strings.behindWheel, value: e.duration(t.drive_duration_s) },
    { label: e.strings.energy, value: e.energy(t.energy_used_kwh) },
    { label: e.strings.efficiency, value: r == null ? "—" : e.unit(r, 0, "Wh/km") }
  ];
}
class Me extends _ {
  constructor() {
    super(P(ue));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "variant", "title", "label", "link-href", "link-label"];
  }
  paint(e) {
    const r = I(this);
    let i = j(this);
    i && !Array.isArray(i) && i.drive_count != null && (i = _e(i, r)), Array.isArray(i) || (i = []);
    const a = this.getAttribute("variant") || "strip";
    if (a === "card") {
      const l = u("div", { class: "card-head" }, e);
      u("h2", { text: this.getAttribute("title") || "" }, l);
      const o = this.getAttribute("link-href"), s = this.getAttribute("link-label");
      o && s && u("a", { class: "text-link", href: o, text: s }, l);
    }
    const n = u("dl", {
      class: a === "strip" ? "metrics summary" : "metrics",
      "aria-label": this.getAttribute("label") || this.getAttribute("title") || ""
    }, e);
    for (const l of i) {
      const o = u("div", null, n);
      u("dt", { text: l.label }, o), u("dd", { text: l.value }, o);
    }
  }
}
class Ce extends _ {
  constructor() {
    super(P(de));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "label"];
  }
  paint(e) {
    const r = this.getAttribute("label") || "";
    this.hasAttribute("role") || this.setAttribute("role", "navigation"), r && this.getAttribute("aria-label") !== r && this.setAttribute("aria-label", r);
    const i = j(this) || [];
    for (const a of i) {
      const n = u("a", { href: a.href || "#", text: a.label || "" }, e);
      a.current && n.setAttribute("aria-current", "page"), n.addEventListener("click", (l) => {
        Y(this, "tl-change", { id: a.id, href: a.href || "" }), a.href || l.preventDefault();
      });
    }
  }
}
class Ne extends _ {
  constructor() {
    super(P(he)), this.addEventListener("keydown", (e) => {
      const r = [...this.shadowRoot.querySelectorAll("[role='tab']")], i = r.indexOf(this.shadowRoot.activeElement);
      if (i < 0) return;
      let a = null;
      e.key === "ArrowRight" && (a = r[(i + 1) % r.length]), e.key === "ArrowLeft" && (a = r[(i - 1 + r.length) % r.length]), e.key === "Home" && (a = r[0]), e.key === "End" && (a = r[r.length - 1]), a && (e.preventDefault(), a.focus());
    });
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "label"];
  }
  paint(e) {
    const r = this.getAttribute("label") || "";
    this.getAttribute("role") !== "tablist" && this.setAttribute("role", "tablist"), r && this.getAttribute("aria-label") !== r && this.setAttribute("aria-label", r);
    const i = j(this) || [];
    for (const a of i) {
      const n = u("a", {
        role: "tab",
        id: a.id || "",
        href: a.href || "#",
        text: a.label || ""
      }, e);
      n.setAttribute("aria-selected", a.selected ? "true" : "false"), a.selected && n.setAttribute("aria-current", "page"), n.addEventListener("click", () => Y(this, "tl-change", { id: a.id, href: a.href || "" }));
    }
  }
}
class Se extends _ {
  constructor() {
    super(P(oe)), this._lat = null, this._lon = null, this._headingProp = null, this._tiles = !1;
  }
  set latitude(e) {
    this._lat = e == null ? null : Number(e), this.isConnected && !this._rendering && this.render();
  }
  get latitude() {
    return this._lat;
  }
  set longitude(e) {
    this._lon = e == null ? null : Number(e), this.isConnected && !this._rendering && this.render();
  }
  get longitude() {
    return this._lon;
  }
  set heading(e) {
    this._headingProp = e == null ? null : Number(e), this.isConnected && !this._rendering && this.render();
  }
  get heading() {
    return this._headingProp;
  }
  paint(e) {
    var v;
    const r = I(this), i = { ...j(this) || {} }, a = Number.isFinite(this._lat) && Number.isFinite(this._lon);
    a && i.empty !== !0 && !i.roads && (i.empty = !1);
    const n = Number(i.width) || 360, l = Number(i.height) || 220, o = n / 2, s = l / 2, c = u("div", { class: "map-stack" }, e), d = u("div", { class: "tiles" }, c), h = document.createElement("slot");
    h.name = "tiles", d.append(h);
    const g = h.assignedNodes({ flatten: !0 }).some((m) => m.nodeType === 1);
    this.classList.toggle("has-tiles", g), g !== this._tiles && (this._tiles = g), h.addEventListener("slotchange", () => {
      const m = h.assignedNodes({ flatten: !0 }).some((M) => M.nodeType === 1);
      m === this._tiles || this._rendering || (this._tiles = m, this.render());
    });
    const f = i.empty !== !1 && !((v = i.roads) != null && v.length) && !a, A = !!i.moving;
    let b = "mini-map";
    A && (b += " live"), f && (b += " empty");
    const w = i.label || r.strings.unknownPosition, k = x("svg", {
      class: b,
      viewBox: `0 0 ${n} ${l}`,
      role: "img",
      "aria-label": w,
      focusable: "false"
    }, c), C = x("defs", null, k), E = x("pattern", { id: "dots", width: "18", height: "18", patternUnits: "userSpaceOnUse" }, C);
    if (x("circle", { class: "map-dot", cx: "1.5", cy: "1.5", r: "1" }, E), !f) {
      const m = x("radialGradient", { id: "fade", cx: "0.5", cy: "0.5", r: "0.62" }, C);
      x("stop", { offset: "0.55", "stop-color": "#000", "stop-opacity": "0" }, m), x("stop", { offset: "1", "stop-color": "#000", "stop-opacity": "1" }, m);
    }
    if (g || (x("rect", { class: "map-bg", width: String(n), height: String(l) }, k), x("rect", { class: "map-dots", width: String(n), height: String(l), fill: "url(#dots)" }, k)), !f) {
      for (const m of i.roads || [])
        m && x("path", { class: "map-road", d: m }, k);
      x("rect", { class: "map-vignette", width: String(n), height: String(l), fill: "url(#fade)" }, k);
    }
    if (x("circle", { class: "map-pulse", cx: String(o), cy: String(s), r: "26" }, k), x("circle", { class: "map-halo", cx: String(o), cy: String(s), r: "15" }, k), !f) {
      const m = i.heading ?? this._headingProp;
      if (m != null && Number.isFinite(Number(m)) && x("path", {
        class: "map-heading",
        d: `M${o} ${(s - 22).toFixed(1)} l7 13 l-7 -4 l-7 4 z`,
        transform: `rotate(${Math.round(Number(m))} ${o} ${s})`
      }, k), x("circle", { class: "map-car", cx: String(o), cy: String(s), r: "7" }, k), i.scale_label) {
        const M = x("g", { class: "map-scale", transform: `translate(16 ${Math.round(l - 16)})` }, k), L = Number(i.scale_width) || 0;
        x("path", { d: `M0 -4 V0 H${L.toFixed(1)} V-4` }, M), x("text", { x: (L + 6).toFixed(1), y: "0", text: i.scale_label }, M);
      }
    }
  }
}
function Tt(t, e, r) {
  return e != null && r != null ? `${t.number(Number(e), 0)} → ${t.percent(r)}` : e != null ? t.percent(e) : "—";
}
function $e(t, e) {
  if (t == null || e == null) return null;
  const r = Math.min(Number(t), Number(e)), i = Math.max(Number(t), Number(e)), a = x("svg", {
    class: e >= t ? "soc-bar gain" : "soc-bar use",
    viewBox: "0 0 100 6",
    preserveAspectRatio: "none",
    "aria-hidden": "true",
    focusable: "false"
  });
  return x("rect", { class: "soc-track", x: "0", y: "0", width: "100", height: "6", rx: "3" }, a), x("rect", {
    class: "soc-fill",
    x: Math.max(0, Math.min(100, r)).toFixed(1),
    y: "0",
    width: Math.max(1.5, i - r).toFixed(1),
    height: "6",
    rx: "3"
  }, a), a;
}
function zt(t, e, r) {
  const i = u("span", { class: "trip-times" }, t);
  if (u("time", { datetime: e.start_at || "", text: e.start_at ? r.clock(e.start_at) : "—" }, i), e.end_at) u("span", { text: r.clock(e.end_at) }, i);
  else {
    const a = u("span", { class: "live" }, i);
    u("span", { class: "live-dot" }, a), a.append(document.createTextNode(r.strings.inProgress));
  }
  return i;
}
function Lt(t, e) {
  !e || e === "poll" || e === "stream" || u("span", { class: "chip source", text: e }, t);
}
function Ft(t) {
  if (t) return u("a", { class: "trip", href: t });
  const e = u("div", { class: "trip" });
  return e.tabIndex = 0, e.setAttribute("role", "link"), e;
}
class Ee extends _ {
  constructor() {
    super(P(kt));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "href"];
  }
  set href(e) {
    e ? this.setAttribute("href", e) : this.removeAttribute("href");
  }
  get href() {
    return this.getAttribute("href");
  }
  paint(e) {
    const r = I(this), i = j(this) || {}, a = this.getAttribute("href") || i.href || "", n = G(i.start_address, i.start_geofence), l = G(i.end_address, i.end_geofence), o = n ? n.name : r.strings.unknownPlace, s = i.end_at ? l ? l.name : r.strings.unknownPlace : r.strings.enRoute, c = Ft(a);
    c.classList.add("trip");
    const d = i.start_at ? r.dayLabel(i.start_at, ot(this)) : "", h = i.start_at ? r.clock(i.start_at) : "";
    c.setAttribute("aria-label", `${d} ${h}, ${o}, ${s}, ${r.distance(i.distance_km)}`.trim()), zt(c, i, r);
    const g = u("span", { class: "route" }, c), f = u("span", { class: "route-stop start", text: o }, g);
    n != null && n.detail && (f.title = n.detail);
    const A = u("span", { class: "route-stop end", text: s }, g);
    l != null && l.detail && i.end_at && (A.title = l.detail);
    const b = u("span", { class: "trip-figures" }, c);
    u("span", { class: "figure", text: r.distance(i.distance_km) }, b), u("span", { class: "muted", text: r.duration(i.duration_s) }, b);
    const w = i.energy_kwh != null ? i.energy_kwh : i.energy_used_kwh, k = i.energy_kwh != null && i.energy_used_kwh == null;
    let C = w == null ? "—" : r.energy(w);
    k && w != null && (C = `≈${C}`);
    const E = st(Number(w), Number(i.distance_km)), v = u("span", { class: "trip-stats" }, c), m = u("span", null, v);
    u("span", { class: "stat-label", text: r.strings.energy }, m), m.append(document.createTextNode(C));
    const M = u("span", null, v);
    u("span", { class: "stat-label", text: r.strings.efficiency }, M), M.append(document.createTextNode(E == null ? "—" : r.unit(E, 0, "Wh/km")));
    const L = u("span", null, v);
    u("span", { class: "stat-label", text: r.strings.battery }, L), L.append(document.createTextNode(Tt(r, i.start_battery_level, i.end_battery_level))), Lt(c, i.source), a || (c.addEventListener("click", () => Y(this, "tl-open", { id: i.id })), c.addEventListener("keydown", (p) => {
      (p.key === "Enter" || p.key === " ") && (p.preventDefault(), Y(this, "tl-open", { id: i.id }));
    })), e.append(c);
  }
}
class Te extends _ {
  constructor() {
    super(P(kt));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "href"];
  }
  set href(e) {
    e ? this.setAttribute("href", e) : this.removeAttribute("href");
  }
  get href() {
    return this.getAttribute("href");
  }
  paint(e) {
    const r = I(this), i = j(this) || {}, a = this.getAttribute("href") || i.href || "", n = G(i.address, i.geofence), l = n ? n.name : r.strings.unknownPlace, o = i.is_dc === !0, s = i.energy_added_kwh, c = s == null ? r.strings.inProgress.replace(/^./, (M) => M.toLocaleUpperCase(r.locale)) : `+${r.energy(s)}`, d = Ft(a);
    d.classList.add("charge"), o && d.classList.add("dc"), d.setAttribute("aria-label", `${i.start_at ? r.clock(i.start_at) : ""} ${l} ${c}`.trim()), zt(d, i, r);
    const h = u("span", { class: "route single" }, d), g = u("span", { class: "route-stop charge-stop", text: l }, h);
    n != null && n.detail && (g.title = n.detail);
    const f = u("span", { class: "muted kind" }, h);
    o && u("span", { class: "chip dc", text: "DC" }, f);
    const A = i.charger_type || (i.is_dc === !1 ? "AC" : i.is_dc == null ? r.strings.charging : "");
    A && f.append(document.createTextNode(o ? ` ${A}` : A));
    const b = u("span", { class: "trip-figures" }, d);
    u("span", { class: "figure", text: c }, b);
    const w = i.duration_s ?? (i.start_at ? Math.max(0, Math.round(((i.end_at ? new Date(i.end_at) : new Date(ot(this))) - new Date(i.start_at)) / 1e3)) : null);
    u("span", { class: "muted", text: r.duration(w) }, b);
    const k = u("span", { class: "trip-stats" }, d), C = u("span", { class: "soc" }, k);
    u("span", { class: "stat-label", text: r.strings.battery }, C), C.append(document.createTextNode(Tt(r, i.start_battery_level, i.end_battery_level)));
    const E = $e(i.start_battery_level, i.end_battery_level);
    E && C.append(E);
    const v = u("span", null, k);
    u("span", { class: "stat-label", text: r.strings.maxPower }, v), v.append(document.createTextNode(r.unit(i.max_power_kw, 0, "kW")));
    const m = u("span", null, k);
    u("span", { class: "stat-label", text: r.strings.cost }, m), m.append(document.createTextNode(r.money(i.cost))), Lt(d, i.source), a || d.addEventListener("click", () => Y(this, "tl-open", { id: i.id })), e.append(d);
  }
}
class ze extends _ {
  constructor() {
    super(P(le));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "heading", "summary"];
  }
  paint(e) {
    const r = this.getAttribute("heading") || "", i = u("section", { class: "day", "aria-label": r }, e), a = u("header", { class: "day-head" }, i);
    u("h2", { text: r }, a), u("p", { class: "muted", text: this.getAttribute("summary") || "" }, a), u("div", { class: "trips", role: "list" }, i).append(document.createElement("slot"));
  }
}
function Le(t, e, r) {
  if (!t) return null;
  if (t.when || t.primary) return t;
  if (!t.start_at) return null;
  const i = !t.end_at;
  if (t.energy_added_kwh != null || t.is_dc != null || t.charger_type) {
    const s = G(t.address, t.geofence);
    return {
      href: t.href || "",
      when: `${e.dayLabel(t.start_at, r)} · ${e.clock(t.start_at)}`,
      from: null,
      to: s ? s.name : e.strings.unknownPlace,
      primary: t.energy_added_kwh == null ? "—" : `+${e.energy(t.energy_added_kwh)}`,
      secondary: e.duration(t.duration_s),
      open: i
    };
  }
  const a = G(t.start_address, t.start_geofence), n = G(t.end_address, t.end_geofence), l = st(Number(t.energy_used_kwh), Number(t.distance_km)), o = [e.duration(t.duration_s)];
  return l != null && o.push(e.unit(l, 0, "Wh/km")), {
    href: t.href || "",
    when: `${e.dayLabel(t.start_at, r)} · ${e.clock(t.start_at)}`,
    from: a ? a.name : e.strings.unknownPlace,
    to: i ? e.strings.enRoute : n ? n.name : e.strings.unknownPlace,
    primary: e.distance(t.distance_km),
    secondary: o.join(" · "),
    open: i
  };
}
class Fe extends _ {
  constructor() {
    super(P(ce));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "title", "empty", "link-href", "link-label"];
  }
  paint(e) {
    const r = I(this), i = Le(j(this), r, ot(this)), a = u("div", { class: "card-head" }, e);
    u("h2", { text: this.getAttribute("title") || "" }, a);
    const n = this.getAttribute("link-href"), l = this.getAttribute("link-label");
    if (n && l && u("a", { class: "text-link", href: n, text: l }, a), !i) {
      u("p", { class: "muted", text: this.getAttribute("empty") || "" }, e);
      return;
    }
    const o = u("a", { class: "recent-link", href: i.href || "#" }, e), s = u("p", { class: "recent-when" }, o);
    s.append(document.createTextNode(i.when || "")), i.open && (s.append(document.createTextNode(" · ")), u("span", { class: "live-dot" }, s), s.append(document.createTextNode(` ${r.strings.inProgress}`)));
    const c = u("div", { class: i.from ? "route" : "route single" }, o);
    i.from && u("span", { class: "route-stop start", text: i.from }, c), u("span", { class: "route-stop end", text: i.to || "" }, c);
    const d = u("p", { class: "recent-metrics" }, o);
    u("span", { class: "figure", text: i.primary || "" }, d), u("span", { class: "muted", text: i.secondary || "" }, d);
  }
}
function De(t, e) {
  if (t && t.status_label)
    return {
      label: t.status_label,
      tone: t.status_tone || "idle",
      charging: !!t.charging
    };
  const r = String((t == null ? void 0 : t.activity) || "").toLowerCase(), i = String((t == null ? void 0 : t.charging_state) || "").toLowerCase();
  let a = "unknown";
  r === "driving" ? a = "driving" : r === "charging" || i === "charging" || i === "starting" ? a = "charging" : r === "updating" ? a = "updating" : r === "asleep" ? a = "asleep" : r === "offline" ? a = "offline" : (t == null ? void 0 : t.plugged_in) === !0 || i === "complete" || i === "stopped" || i === "nopower" ? a = "plugged" : (r === "online" || r === "parked") && (a = "parked");
  const n = {
    driving: ["drive", e.driving],
    charging: ["charge", e.charging],
    plugged: ["charge", e.plugged],
    parked: ["park", e.parked],
    updating: ["note", e.updating],
    asleep: ["idle", e.asleep],
    offline: ["idle", e.offline],
    unknown: ["idle", e.unknown]
  }, [l, o] = n[a];
  return { label: o, tone: l, charging: a === "charging" };
}
function vt(t, e) {
  return e == null || !Number.isFinite(Number(e)) ? null : `${t.number(Number(e), 1)}${t.sep}°C`;
}
function Pe(t, e) {
  if (Array.isArray(t.facts)) return t.facts;
  const r = e.locale.toLowerCase().startsWith("fr"), i = [];
  typeof t.locked == "boolean" && i.push({
    label: r ? "Portes" : "Doors",
    value: t.locked ? r ? "Verrouillées" : "Locked" : r ? "Déverrouillées" : "Unlocked"
  });
  const a = vt(e, t.inside_temp_c);
  a && i.push({ label: r ? "Intérieur" : "Inside", value: a });
  const n = vt(e, t.outside_temp_c);
  return n && i.push({ label: r ? "Extérieur" : "Outside", value: n }), t.odometer_km != null && i.push({ label: r ? "Compteur" : "Odometer", value: e.unit(t.odometer_km, 0, e.strings.km) }), t.sentry_mode === !0 && i.push({ label: r ? "Sentinelle" : "Sentry", value: r ? "Active" : "On" }), t.software_version && i.push({ label: r ? "Logiciel" : "Software", value: String(t.software_version) }), i;
}
class je extends _ {
  constructor() {
    super(P(ae));
  }
  static get observedAttributes() {
    return [..._.observedAttributes, "image-url", "alt", "unavailable"];
  }
  set imageUrl(e) {
    e ? this.setAttribute("image-url", e) : this.removeAttribute("image-url");
  }
  get imageUrl() {
    return this.getAttribute("image-url");
  }
  paint(e) {
    const r = I(this), i = j(this) || {}, a = De(i, r.strings), n = u("div", { class: "hero-head" }, e), l = u("div", null, n), o = u("p", { class: "eyebrow" }, l), s = i.model || r.strings.vehicle;
    o.textContent = i.trim ? `${s} · ${i.trim}` : s, u("h2", { class: "hero-name", id: "hero-name", text: i.name || r.strings.vehicle }, l);
    const c = u("span", { class: `chip status ${a.tone}` }, n);
    u("span", { class: "status-dot", "aria-hidden": "true" }, c), c.append(document.createTextNode(a.label));
    const d = u("div", { class: "hero-car" }, e), h = new At(), g = this.getAttribute("image-url") || i.image_url || "";
    g && (h.imageUrl = g);
    const f = this.getAttribute("alt") || i.name || r.strings.vehicle;
    h.setAttribute("alt", f);
    const A = this.getAttribute("unavailable");
    A && h.setAttribute("unavailable", A), h.setAttribute("locale", r.locale), this.getAttribute("theme") && h.setAttribute("theme", this.getAttribute("theme")), d.append(h);
    const b = u("div", { class: "hero-energy" }, e), w = u("p", { class: "hero-battery" }, b), k = i.battery_text || (i.battery_level == null ? "—" : r.percent(i.battery_level));
    u("span", { class: "hero-figure", text: k }, w), u("span", { class: "hero-label", text: r.strings.battery }, w);
    const C = u("p", { class: "hero-range" }, b), E = i.range_text || (i.rated_range_km == null ? "—" : r.distance(i.rated_range_km));
    u("span", { class: "hero-figure", text: E }, C), u("span", { class: "hero-label", text: r.strings.range }, C);
    const v = document.createElement("tl-battery");
    i.battery_level != null && (v.level = i.battery_level), i.charge_limit_soc != null && (v.limit = i.charge_limit_soc), v.charging = a.charging, v.setAttribute("locale", r.locale), e.append(v), i.note && u("p", { class: "hero-note", text: i.note }, e);
    const m = Pe(i, r);
    if (m.length) {
      const M = u("dl", { class: "hero-facts" }, e);
      for (const L of m) {
        const p = u("div", null, M);
        u("dt", { text: L.label }, p), u("dd", { text: L.value }, p);
      }
    }
  }
}
const Ue = [
  ["tl-battery", pe],
  ["tl-car-image", At],
  ["tl-vehicle-card", je],
  ["tl-mini-map", Se],
  ["tl-drive-row", Ee],
  ["tl-charge-row", Te],
  ["tl-day-group", ze],
  ["tl-recent-card", Fe],
  ["tl-summary-strip", Me],
  ["tl-period-chips", Ce],
  ["tl-tabs", Ne],
  ["tl-chart", Ae]
];
function Be(t = customElements) {
  for (const [e, r] of Ue)
    t.get(e) || t.define(e, r);
}
Be();
export {
  at as NNBSP,
  pe as TlBattery,
  At as TlCarImage,
  Te as TlChargeRow,
  Ae as TlChart,
  ze as TlDayGroup,
  Ee as TlDriveRow,
  Se as TlMiniMap,
  Ce as TlPeriodChips,
  Fe as TlRecentCard,
  Me as TlSummaryStrip,
  Ne as TlTabs,
  je as TlVehicleCard,
  Yt as dayLabel,
  Be as defineElements,
  Wt as duration,
  te as makeFormat,
  Et as mountChart,
  Oe as mountCharts,
  It as number,
  Bt as stringsFor
};
