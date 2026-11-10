// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! QR drawing for the authenticator setup page.
//!
//! The graphic is built here. Nothing is sent to an external service.

use std::fmt::Write;

use qrcode::QrCode;
use qrcode::types::Color;

/// Inline SVG with a quiet zone. It has no stylesheet URL and no image URL.
pub(crate) fn svg(data: &str) -> Option<String> {
    let code = QrCode::new(data.as_bytes()).ok()?;
    let side = code.width();
    let quiet = 2_usize;
    let full = side + quiet * 2;
    let colors = code.to_colors();
    let mut marks = String::new();
    for y in 0..side {
        for x in 0..side {
            if colors[y * side + x] != Color::Dark {
                continue;
            }
            let left = x + quiet;
            let top = y + quiet;
            let _ = write!(
                marks,
                r#"<rect x="{left}" y="{top}" width="1" height="1"/>"#
            );
        }
    }
    Some(format!(
        r##"<svg class="qr" viewBox="0 0 {full} {full}" width="220" height="220" role="img" aria-label="Code QR pour l'application d'authentification"><rect width="{full}" height="{full}" fill="#ffffff"/><g fill="#111318">{marks}</g></svg>"##
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn authenticator_qr_is_inline_and_has_no_external_url() {
        let svg =
            super::svg("otpauth://totp/Tesla%20Logger:admin?secret=ABCDEF&issuer=Tesla%20Logger")
                .expect("qr");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(!svg.contains("http://"));
        assert!(!svg.contains("https://"));
        assert!(!svg.contains("style="));
    }
}
