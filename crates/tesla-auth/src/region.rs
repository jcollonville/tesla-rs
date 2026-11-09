// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;
use std::str::FromStr;

use url::Url;

use crate::AuthError;

/// Fleet API region.
///
/// North America and Europe share an authorization server and use different audiences.
/// China uses a separate account, authorization server, and API host.
///
/// See <https://developer.tesla.com/docs/fleet-api/getting-started/regions-countries>.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Region {
    /// North America and Asia-Pacific, excluding China.
    NorthAmerica,
    /// Europe, the Middle East, and Africa.
    Europe,
    /// China. Requires a separate application created on developer.tesla.cn.
    China,
}

impl Region {
    /// Fleet API origin used as the OAuth `audience` and as the API base URL.
    #[must_use]
    pub const fn fleet_api_base(self) -> &'static str {
        match self {
            Self::NorthAmerica => "https://fleet-api.prd.na.vn.cloud.tesla.com",
            Self::Europe => "https://fleet-api.prd.eu.vn.cloud.tesla.com",
            Self::China => "https://fleet-api.prd.cn.vn.cloud.tesla.cn",
        }
    }

    /// OAuth audience. Tesla requires this to be a Fleet API base URL.
    #[must_use]
    pub const fn audience(self) -> &'static str {
        self.fleet_api_base()
    }

    /// Browser authorization endpoint.
    #[must_use]
    pub const fn authorize_endpoint(self) -> &'static str {
        match self {
            Self::NorthAmerica | Self::Europe => "https://auth.tesla.com/oauth2/v3/authorize",
            Self::China => "https://auth.tesla.cn/oauth2/v3/authorize",
        }
    }

    /// Token endpoint used by application servers.
    ///
    /// For North America and Europe this is `fleet-auth.prd.vn.cloud.tesla.com`, which the
    /// Fleet API auth guide requires for server-side token calls. The OpenID discovery
    /// document still advertises `auth.tesla.com`; this crate follows the auth guide.
    /// China uses `auth.tesla.cn`, matching the Chinese third-party token guide.
    #[must_use]
    pub const fn token_endpoint(self) -> &'static str {
        match self {
            Self::NorthAmerica | Self::Europe => {
                "https://fleet-auth.prd.vn.cloud.tesla.com/oauth2/v3/token"
            }
            Self::China => "https://auth.tesla.cn/oauth2/v3/token",
        }
    }

    /// Consent-revocation page.
    ///
    /// The English guide documents the `auth.tesla.com` URL. The China URL uses the China
    /// authorization host; Tesla's Chinese guide does not restate this page separately.
    #[must_use]
    pub const fn revoke_consent_endpoint(self) -> &'static str {
        match self {
            Self::NorthAmerica | Self::Europe => "https://auth.tesla.com/user/revoke/consent",
            Self::China => "https://auth.tesla.cn/user/revoke/consent",
        }
    }

    /// Short code returned by `GET /api/1/users/region` (`na`, `eu`, `cn`).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NorthAmerica => "na",
            Self::Europe => "eu",
            Self::China => "cn",
        }
    }

    /// Matches a Fleet API base URL, ignoring a trailing slash and ASCII case.
    #[must_use]
    pub fn from_fleet_api_base(value: &str) -> Option<Self> {
        let trimmed = value.trim().trim_end_matches('/');
        [Self::NorthAmerica, Self::Europe, Self::China]
            .into_iter()
            .find(|region| trimmed.eq_ignore_ascii_case(region.fleet_api_base()))
    }

    /// Builds the consent-management URL documented for third-party tokens.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidUrl`] when the regional endpoint cannot be parsed.
    pub fn revoke_consent_url(self, client_id: &str, back_url: &str) -> Result<Url, AuthError> {
        let mut url = Url::parse(self.revoke_consent_endpoint())?;
        url.query_pairs_mut()
            .append_pair("revoke_client_id", client_id)
            .append_pair("back_url", back_url);
        Ok(url)
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Region {
    type Err = AuthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "na" | "north-america" | "north_america" => Ok(Self::NorthAmerica),
            "eu" | "europe" => Ok(Self::Europe),
            "cn" | "china" => Ok(Self::China),
            _ => Err(AuthError::UnknownRegion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Region;

    #[test]
    fn documents_production_urls() {
        assert_eq!(
            Region::NorthAmerica.fleet_api_base(),
            "https://fleet-api.prd.na.vn.cloud.tesla.com"
        );
        assert_eq!(
            Region::Europe.fleet_api_base(),
            "https://fleet-api.prd.eu.vn.cloud.tesla.com"
        );
        assert_eq!(
            Region::China.fleet_api_base(),
            "https://fleet-api.prd.cn.vn.cloud.tesla.cn"
        );
        assert_eq!(
            Region::Europe.token_endpoint(),
            "https://fleet-auth.prd.vn.cloud.tesla.com/oauth2/v3/token"
        );
        assert_eq!(
            Region::China.authorize_endpoint(),
            "https://auth.tesla.cn/oauth2/v3/authorize"
        );
        assert_eq!(
            Region::China.token_endpoint(),
            "https://auth.tesla.cn/oauth2/v3/token"
        );
        assert_eq!(Region::Europe.audience(), Region::Europe.fleet_api_base());
    }

    #[test]
    fn parses_region_codes_and_base_urls() {
        assert_eq!("eu".parse::<Region>().unwrap(), Region::Europe);
        assert_eq!(
            Region::from_fleet_api_base("https://fleet-api.prd.na.vn.cloud.tesla.com/"),
            Some(Region::NorthAmerica)
        );
        assert!("us".parse::<Region>().is_err());
    }
}
