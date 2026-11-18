# `@jcollonville/tesla-logger-ui`

Shared presentation for the Tesla Logger console and the Home Assistant card. Custom elements, no network calls, no authentication. The console serves the built bundle from the same origin. The Home Assistant card must vendor it at **build time**. It must not load the script from a Tesla Logger server at runtime.

The package is `MIT OR Apache-2.0`, the same licence as the library crates. `tesla-logger` itself stays AGPL-3.0-only and may embed this package. The built bundle has no third-party runtime code, so a Home Assistant card can vendor it under MIT. `vite` and `jsdom` are development dependencies only and are not included in the published tarball.

## Elements

| Element | Data | Notes |
| --- | --- | --- |
| `tl-vehicle-card` | Vehicle state JSON | Official render via `image-url` only. Missing, 404, or a failed load shows `unavailable` (default follows `locale`). |
| `tl-car-image` | `image-url` | Same image behaviour, for the fleet card. |
| `tl-battery` | `level`, `limit`, `charging` | Charge-limit tick. |
| `tl-mini-map` | Projected paths in viewBox units | No tile fetch. Optional `slot="tiles"`. `latitude` / `longitude` are JS properties and are not written into the DOM. |
| `tl-drive-row` | `DriveJson` plus `energy_kwh`, `source` | `href` attribute. |
| `tl-charge-row` | `ChargeJson` | |
| `tl-day-group` | `heading`, `summary` attributes | Default slot is the rows. |
| `tl-recent-card` | Recent drive or charge, raw or preformatted | |
| `tl-summary-strip` | `[{label, value}]` or `StatsJson` | `variant="strip"` or `"card"`. |
| `tl-period-chips` | `[{id, label, href, current}]` | Fires `tl-change`. |
| `tl-tabs` | `[{id, label, href, selected}]` | Fires `tl-change`. Arrow keys move focus. |
| `tl-chart` | Bar or line payload used by the console (`data-chart`) | Same chart engine as the drive and charge detail pages. |

Every element accepts:

- `locale` (BCP 47). French formatting matches the console. Anything else uses `Intl`, with English strings when the locale is not French.
- `time-zone` (IANA).
- `theme`: `light`, `dark`, or omit to inherit tokens.
- `format`: a property (not an attribute) of optional formatters. See `FormatHooks` in `index.d.ts`.
- `strings`: a property that overrides individual labels.
- `data`: a property. Server-rendered pages may instead include `<script type="application/json">`.

JSON field names are the `/api/v1` snake_case names. Display strings such as `battery_text` win over raw numbers when both are present, which is how the console stays on its own formatter.

## Tokens

`tokens.css` defines `--tl-*` for light and dark (`prefers-color-scheme`). The console aliases the same names to its existing variables. A Home Assistant theme can set them on the card:

```css
ha-card {
  --tl-bg: var(--primary-background-color);
  --tl-surface: var(--card-background-color);
  --tl-text: var(--primary-text-color);
  --tl-muted: var(--secondary-text-color);
  --tl-accent: var(--primary-color);
  --tl-line: var(--divider-color);
}
```

Set `theme="dark"` when the card should ignore the OS scheme.

## Install in ha-tesla-logger

Releases are GitHub release assets, not a runtime script and not GitHub Packages (that registry requires a token even for public packages).

1. Pick a tag `ui-vX.Y.Z` (the `package.json` version with an `ui-v` prefix).
2. Download the tarball and `SHA256SUMS` from that release.
3. Check the sum, then install the tarball:

```bash
sha256sum -c SHA256SUMS
npm install ./jcollonville-tesla-logger-ui-X.Y.Z.tgz
```

Or, in `package.json`, pin the release asset directly:

```json
{
  "dependencies": {
    "@jcollonville/tesla-logger-ui": "https://github.com/jcollonville/tesla-rs/releases/download/ui-v0.1.0/jcollonville-tesla-logger-ui-0.1.0.tgz"
  }
}
```

Verify that URL's bytes against `SHA256SUMS` in CI before publishing the card.

```js
import "@jcollonville/tesla-logger-ui";
import "@jcollonville/tesla-logger-ui/tokens.css";

const card = document.createElement("tl-vehicle-card");
card.locale = hass.locale.language;
card.timeZone = hass.locale.time_zone || "UTC";
card.imageUrl = signedProxyUrl;
card.unavailable = hass.localize("ui.tesla_logger.image_unavailable");
card.data = stateFromApi;
```

`defineElements()` runs on import. The card bundle must include this module. Do not add a `<script src="https://…tesla-logger…">`.

Until the first `ui-v*` tag exists, build a tarball from this directory:

```bash
npm ci
npm test
npm run build
npm pack
```

## Develop

```bash
cd ui
npm ci
npm test
npm run build
```

`server/` embeds `dist/tesla-logger-ui.js`. CI fails if that file drifts from the source. Open `demo/index.html` via a static server (`npx serve ui`) to look at fixtures. Fixtures use invented place names and projected map paths. They contain no VIN, token, address, coordinate, or Tesla render.
