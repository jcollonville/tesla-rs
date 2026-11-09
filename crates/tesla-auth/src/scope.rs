// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::AuthError;

/// A Fleet API OAuth scope.
///
/// Names match the [authentication overview](https://developer.tesla.com/docs/fleet-api/authentication/overview)
/// plus `email` and `profile` from the third-party OpenID configuration.
/// Unknown names are preserved so a newer scope is not dropped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Scope {
    /// Sign in with Tesla.
    OpenId,
    /// Refresh token without asking the user to sign in again.
    OfflineAccess,
    /// Profile, contact, address, and referral information.
    UserData,
    /// Live vehicle data, service data, and nearby chargers.
    VehicleDeviceData,
    /// Precise and coarse vehicle location.
    VehicleLocation,
    /// Vehicle commands such as unlock, remote start, and key management.
    VehicleCmds,
    /// Charging history, billing, and charge control.
    VehicleChargingCmds,
    /// Partner-token scope for detailed vehicle specifications.
    VehicleSpecs,
    /// Partner-token scope for pricing details.
    VehiclePricingInfo,
    /// Energy site live status and history.
    EnergyDeviceData,
    /// Energy site settings.
    EnergyCmds,
    /// Business enterprise-management functions.
    EnterpriseManagement,
    /// OpenID `email` scope advertised by the discovery document.
    Email,
    /// OpenID `profile` scope advertised by the discovery document.
    Profile,
    /// A scope this crate does not special-case.
    Other(String),
}

impl Scope {
    /// Parses one scope name.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidScope`] when `name` is empty or contains
    /// characters outside ASCII alphanumerics, `_`, `-`, `.`, and `:`.
    pub fn new(name: &str) -> Result<Self, AuthError> {
        let name = name.trim();
        if name.is_empty() || !name.chars().all(valid_scope_char) {
            return Err(AuthError::InvalidScope);
        }
        Ok(match name {
            "openid" => Self::OpenId,
            "offline_access" => Self::OfflineAccess,
            "user_data" => Self::UserData,
            "vehicle_device_data" => Self::VehicleDeviceData,
            "vehicle_location" => Self::VehicleLocation,
            "vehicle_cmds" => Self::VehicleCmds,
            "vehicle_charging_cmds" => Self::VehicleChargingCmds,
            "vehicle_specs" => Self::VehicleSpecs,
            "vehicle_pricing_info" => Self::VehiclePricingInfo,
            "energy_device_data" => Self::EnergyDeviceData,
            "energy_cmds" => Self::EnergyCmds,
            "enterprise_management" => Self::EnterpriseManagement,
            "email" => Self::Email,
            "profile" => Self::Profile,
            other => Self::Other(other.to_owned()),
        })
    }

    /// Wire value placed in the `scope` parameter.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OpenId => "openid",
            Self::OfflineAccess => "offline_access",
            Self::UserData => "user_data",
            Self::VehicleDeviceData => "vehicle_device_data",
            Self::VehicleLocation => "vehicle_location",
            Self::VehicleCmds => "vehicle_cmds",
            Self::VehicleChargingCmds => "vehicle_charging_cmds",
            Self::VehicleSpecs => "vehicle_specs",
            Self::VehiclePricingInfo => "vehicle_pricing_info",
            Self::EnergyDeviceData => "energy_device_data",
            Self::EnergyCmds => "energy_cmds",
            Self::EnterpriseManagement => "enterprise_management",
            Self::Email => "email",
            Self::Profile => "profile",
            Self::Other(name) => name,
        }
    }

    /// Space-delimited scope parameter.
    #[must_use]
    pub fn join(scopes: &[Self]) -> String {
        scopes
            .iter()
            .map(Self::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Splits a space-delimited scope string.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidScope`] when any entry is not a valid scope name.
    pub fn parse_list(input: &str) -> Result<Vec<Self>, AuthError> {
        input.split_whitespace().map(Self::new).collect()
    }

    /// Scopes for a third-party application that reads vehicles, location, and charging.
    ///
    /// Includes `openid` and `offline_access`, which Tesla requires in order to return a
    /// refresh token. `vehicle_cmds` is included because the scope description mentions wake,
    /// even though the `wake_up` endpoint reference lists `vehicle_device_data`.
    #[must_use]
    pub fn third_party_vehicle() -> Vec<Self> {
        vec![
            Self::OpenId,
            Self::OfflineAccess,
            Self::UserData,
            Self::VehicleDeviceData,
            Self::VehicleLocation,
            Self::VehicleCmds,
            Self::VehicleChargingCmds,
        ]
    }

    /// Scopes used by Tesla's partner-token example.
    #[must_use]
    pub fn partner_account() -> Vec<Self> {
        vec![
            Self::OpenId,
            Self::VehicleDeviceData,
            Self::VehicleCmds,
            Self::VehicleChargingCmds,
        ]
    }
}

fn valid_scope_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Scope;

    #[test]
    fn joins_and_parses_known_scopes() {
        let scopes = Scope::third_party_vehicle();
        let text = Scope::join(&scopes);
        assert_eq!(Scope::parse_list(&text).unwrap(), scopes);
    }

    #[test]
    fn preserves_unknown_scope() {
        let parsed = Scope::parse_list("openid custom_scope").unwrap();
        assert_eq!(
            parsed,
            vec![Scope::OpenId, Scope::Other("custom_scope".to_owned())]
        );
    }

    #[test]
    fn rejects_empty_and_whitespace_scope() {
        assert!(Scope::new("").is_err());
        assert!(Scope::new("vehicle data").is_err());
    }
}
