// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Custom elements shared with `@jcollonville/tesla-logger-ui`.
//!
//! The element renders in the browser. The server only supplies attributes and
//! a JSON payload. `script type="application/json"` is not executed, so it stays
//! inside the strict script policy.

use maud::{Markup, PreEscaped, html};
use serde::Serialize;

#[allow(clippy::needless_pass_by_value)]
pub(super) fn element(
    tag: &str,
    attrs: &[(&str, &str)],
    json: Option<&str>,
    inner: Markup,
) -> Markup {
    let mut open = format!("<{tag}");
    for (name, value) in attrs {
        open.push(' ');
        open.push_str(name);
        open.push_str("=\"");
        open.push_str(&escape_attr(value));
        open.push('"');
    }
    open.push('>');
    let close = format!("</{tag}>");
    html! {
        (PreEscaped(open))
        @if let Some(json) = json {
            script type="application/json" { (PreEscaped(json)) }
        }
        (inner)
        (PreEscaped(close))
    }
}

pub(super) fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".to_owned())
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub(super) fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
