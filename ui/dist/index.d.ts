export function defineElements(registry?: CustomElementRegistry): void;
export function mountCharts(root?: ParentNode): void;
export function mountChart(figure: HTMLElement, data: ChartData): unknown;
export const NNBSP: string;
export function number(value: number, decimals: number): string;
export function duration(seconds: number | null, strings: Strings, sep?: string): string;
export function dayLabel(iso: string, timeZone: string, nowIso: string, strings: Strings, locale: string): string;
export function stringsFor(locale: string): Strings;
export function makeFormat(options?: FormatOptions): Formatter;

export interface Strings {
  today: string;
  yesterday: string;
  unavailableImage: string;
  unknownPlace: string;
  unknownPosition: string;
  enRoute: string;
  inProgress: string;
  battery: string;
  range: string;
  energy: string;
  efficiency: string;
  maxPower: string;
  cost: string;
  drives: string;
  distance: string;
  behindWheel: string;
  driving: string;
  charging: string;
  plugged: string;
  parked: string;
  asleep: string;
  offline: string;
  updating: string;
  unknown: string;
  km: string;
  kwh: string;
  currency: string;
  [key: string]: string | string[];
}

export interface FormatHooks {
  number?: (value: number, decimals: number) => string;
  unit?: (value: number | null, decimals: number, suffix: string) => string;
  distance?: (km: number | null) => string;
  energy?: (kwh: number | null) => string;
  percent?: (value: number | null) => string;
  money?: (value: number | null) => string;
  duration?: (seconds: number | null) => string;
  clock?: (iso: string) => string;
  dayLabel?: (iso: string, now?: string) => string;
  ago?: (iso: string | null, now?: string) => string;
  strings?: Partial<Strings>;
}

export interface FormatOptions {
  locale?: string;
  timeZone?: string;
  format?: FormatHooks;
  strings?: Partial<Strings>;
}

export interface Formatter {
  locale: string;
  timeZone: string;
  strings: Strings;
  number: (value: number, decimals: number) => string;
  unit: (value: number | null, decimals: number, suffix: string) => string;
  distance: (km: number | null) => string;
  energy: (kwh: number | null) => string;
  percent: (value: number | null) => string;
  money: (value: number | null) => string;
  duration: (seconds: number | null) => string;
  clock: (iso: string) => string;
  dayLabel: (iso: string, now?: string) => string;
  ago: (iso: string | null, now?: string) => string;
}

export interface Fact {
  label: string;
  value: string;
}

/** Subset of `/api/v1` vehicle state plus display overrides. */
export interface VehicleCardData {
  name?: string;
  model?: string | null;
  trim?: string | null;
  activity?: string | null;
  charging_state?: string | null;
  plugged_in?: boolean | null;
  battery_level?: number | null;
  charge_limit_soc?: number | null;
  rated_range_km?: number | null;
  battery_text?: string;
  range_text?: string;
  note?: string | null;
  image_url?: string | null;
  status_label?: string;
  status_tone?: string;
  charging?: boolean;
  facts?: Fact[];
  locked?: boolean | null;
  inside_temp_c?: number | null;
  outside_temp_c?: number | null;
  odometer_km?: number | null;
  sentry_mode?: boolean | null;
  software_version?: string | null;
}

export interface DriveData {
  id?: number;
  start_at?: string;
  end_at?: string | null;
  distance_km?: number | null;
  duration_s?: number | null;
  energy_used_kwh?: number | null;
  energy_kwh?: number | null;
  start_battery_level?: number | null;
  end_battery_level?: number | null;
  source?: string;
  start_address?: string | null;
  end_address?: string | null;
  start_geofence?: string | null;
  end_geofence?: string | null;
  href?: string;
  when?: string;
  from?: string | null;
  to?: string;
  primary?: string;
  secondary?: string;
  open?: boolean;
}

export interface ChargeData {
  id?: number;
  start_at?: string;
  end_at?: string | null;
  energy_added_kwh?: number | null;
  duration_s?: number | null;
  cost?: number | null;
  is_dc?: boolean | null;
  charger_type?: string | null;
  max_power_kw?: number | null;
  start_battery_level?: number | null;
  end_battery_level?: number | null;
  source?: string;
  address?: string | null;
  geofence?: string | null;
  href?: string;
}

export interface MiniMapData {
  label?: string;
  moving?: boolean;
  empty?: boolean;
  width?: number;
  height?: number;
  roads?: string[];
  heading?: number | null;
  scale_width?: number;
  scale_label?: string;
}

export interface ChipOption {
  id: string;
  label: string;
  href?: string;
  current?: boolean;
}

export interface TabOption {
  id: string;
  label: string;
  href?: string;
  selected?: boolean;
}

export interface ChartSeries {
  name: string;
  unit: string;
  decimals: number;
  tone: string;
  values: Array<number | null>;
  area?: boolean;
  signed?: boolean;
}

export interface ChartData {
  kind: "line" | "bar";
  title?: string;
  summary?: string;
  labels?: string[];
  titles?: string[];
  hrefs?: string[];
  stacked?: boolean;
  x?: { kind: string; origin_ms?: number; offset_min?: number; unit?: string; label?: string; decimals?: number };
  t?: number[];
  series: ChartSeries[];
}

export interface StatsData {
  drive_count?: number;
  distance_km?: number;
  drive_duration_s?: number;
  energy_used_kwh?: number;
  charge_count?: number;
  energy_added_kwh?: number;
}

interface TlBase extends HTMLElement {
  locale: string | null;
  timeZone: string | null;
  theme: string | null;
  now: string | null;
  format: FormatHooks | null;
  strings: Partial<Strings> | null;
}

export class TlBattery extends TlBase {
  level: number | null;
  limit: number | null;
  charging: boolean;
}
export class TlCarImage extends TlBase {
  imageUrl: string | null;
}
export class TlVehicleCard extends TlBase {
  imageUrl: string | null;
  data: VehicleCardData | null;
}
export class TlMiniMap extends TlBase {
  data: MiniMapData | null;
  /** Not reflected to an attribute. Used only to centre a dot. */
  latitude: number | null;
  longitude: number | null;
  heading: number | null;
}
export class TlDriveRow extends TlBase {
  href: string | null;
  data: DriveData | null;
}
export class TlChargeRow extends TlBase {
  href: string | null;
  data: ChargeData | null;
}
export class TlDayGroup extends TlBase {}
export class TlRecentCard extends TlBase {
  data: DriveData | ChargeData | null;
}
export class TlSummaryStrip extends TlBase {
  data: Fact[] | StatsData | null;
}
export class TlPeriodChips extends TlBase {
  data: ChipOption[] | null;
}
export class TlTabs extends TlBase {
  data: TabOption[] | null;
}
export class TlChart extends TlBase {
  data: ChartData | null;
}

declare global {
  interface HTMLElementTagNameMap {
    "tl-battery": TlBattery;
    "tl-car-image": TlCarImage;
    "tl-vehicle-card": TlVehicleCard;
    "tl-mini-map": TlMiniMap;
    "tl-drive-row": TlDriveRow;
    "tl-charge-row": TlChargeRow;
    "tl-day-group": TlDayGroup;
    "tl-recent-card": TlRecentCard;
    "tl-summary-strip": TlSummaryStrip;
    "tl-period-chips": TlPeriodChips;
    "tl-tabs": TlTabs;
    "tl-chart": TlChart;
  }
}
