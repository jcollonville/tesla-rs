// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

/// Warn when the default partition has rows, or the 2035 horizon is close.
pub(crate) fn storage_alert(default_rows: i64, year: i32) -> Option<&'static str> {
    if default_rows > 0 {
        return Some("positions_default is not empty");
    }
    if year >= 2034 {
        return Some("the 2035 partition horizon is near");
    }
    None
}

pub(crate) fn proxy_ip_allowed(allowlist: &[String], ip: &str) -> bool {
    allowlist.iter().any(|entry| entry == ip)
}

#[cfg(test)]
mod tests {
    use super::{proxy_ip_allowed, storage_alert};

    #[test]
    fn alerts_on_the_default_partition_and_the_horizon() {
        assert_eq!(
            storage_alert(1, 2026),
            Some("positions_default is not empty")
        );
        assert_eq!(
            storage_alert(0, 2034),
            Some("the 2035 partition horizon is near")
        );
        assert_eq!(storage_alert(0, 2026), None);
        assert!(proxy_ip_allowed(&["10.0.0.1".into()], "10.0.0.1"));
        assert!(!proxy_ip_allowed(&["10.0.0.1".into()], "10.0.0.2"));
    }
}
