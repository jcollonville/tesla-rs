// SPDX-License-Identifier: MIT OR Apache-2.0

//! Device identity from a vehicle client certificate.
//!
//! Ported from `messages/identity.go` in `teslamotors/fleet-telemetry` commit
//! `bb7c8a931e46da3b3eb6d20a67541c8f755f1ee6`. The subject common name is the
//! device id (a VIN for vehicles), with `.` replaced by `-`. The issuer common
//! name, and on newer product CAs the extended-key-usage OID, select the device
//! type.

use x509_parser::prelude::*;

use crate::error::TelemetryError;

/// Identity the reference server derives from the mutual-TLS client certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// `vehicle_device`, `energy_device`, and the other reference-server types.
    pub device_type: String,
    /// Subject common name, with `.` replaced by `-`. This is the VIN for vehicles.
    pub device_id: String,
    /// `{device_type}.{device_id}`, the deprecated sender id.
    pub sender_id: String,
}

impl Identity {
    /// Parses a DER-encoded client certificate.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::Certificate`] when the DER is not an X.509
    /// certificate, [`TelemetryError::ExcludedCertificate`] for the wifi EAP
    /// OID, and [`TelemetryError::UntrustedIssuer`] when the issuer is unknown.
    pub fn from_der(der: &[u8]) -> Result<Self, TelemetryError> {
        let (_, cert) = X509Certificate::from_der(der).map_err(|_| TelemetryError::Certificate)?;
        let subject = common_name(cert.subject()).unwrap_or_default();
        let issuer = common_name(cert.issuer()).unwrap_or_default();
        let units = organizational_units(cert.subject());
        let oids = extended_key_usage_oids(&cert)?;
        let device_type = device_type_for(&issuer, &units, &oids)?;
        let device_id = subject.replace('.', "-");
        let sender_id = format!("{device_type}.{device_id}");
        Ok(Self {
            device_type,
            device_id,
            sender_id,
        })
    }
}

pub(crate) fn device_type_for(
    issuer_cn: &str,
    organizational_units: &[String],
    eku_oids: &[String],
) -> Result<String, TelemetryError> {
    if KNOWN_OID_ISSUERS.contains(&issuer_cn) {
        return device_type_from_oid(organizational_units, eku_oids, issuer_cn);
    }
    let Some(mut device_type) = known_issuer(issuer_cn) else {
        return Err(TelemetryError::UntrustedIssuer {
            issuer: issuer_cn.to_owned(),
        });
    };
    if issuer_cn == "Tesla Motors Products CA"
        && organizational_units
            .first()
            .is_some_and(|unit| unit == "Tesla Motors SN")
    {
        device_type = "vehicle_board_device";
    }
    Ok(device_type.to_owned())
}

fn device_type_from_oid(
    organizational_units: &[String],
    eku_oids: &[String],
    issuer_cn: &str,
) -> Result<String, TelemetryError> {
    for oid in eku_oids {
        if oid == "1.3.6.1.4.1.49279.2.5.1.1" {
            return Err(TelemetryError::ExcludedCertificate);
        }
        if let Some(device_type) = oid_device_type(oid) {
            if let Some(override_type) = ou_device_type(organizational_units) {
                return Ok(override_type.to_owned());
            }
            return Ok(device_type.to_owned());
        }
    }
    Err(TelemetryError::UntrustedIssuer {
        issuer: issuer_cn.to_owned(),
    })
}

fn known_issuer(issuer_cn: &str) -> Option<&'static str> {
    match issuer_cn {
        "TeslaMotors" | "Tesla Issuing CA" | "Tesla Motors Products CA" => Some("vehicle_device"),
        _ => None,
    }
}

fn oid_device_type(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "1.3.6.1.4.1.49279.2.4.1" | "1.3.6.1.4.1.49279.2.5.1" => "vehicle_device",
        "1.3.6.1.4.1.49279.2.4.11" | "1.3.6.1.4.1.49279.2.5.11" => "vehicle_board_device",
        "1.3.6.1.4.1.49279.2.4.2" | "1.3.6.1.4.1.49279.2.5.2" => "energy_device",
        "1.3.6.1.4.1.49279.2.4.12" | "1.3.6.1.4.1.49279.2.5.12" => "das_device",
        "1.3.6.1.4.1.49279.2.4.14" | "1.3.6.1.4.1.49279.2.5.14" => "vehicle_service",
        "1.3.6.1.4.1.49279.2.4.15" | "1.3.6.1.4.1.49279.2.5.15" => "das_service",
        "1.3.6.1.4.1.49279.2.4.16" | "1.3.6.1.4.1.49279.2.5.16" => "energy_service",
        "1.3.6.1.4.1.49279.2.4.28" | "1.3.6.1.4.1.49279.2.5.28" => "robotics_device",
        _ => return None,
    })
}

fn ou_device_type(organizational_units: &[String]) -> Option<&'static str> {
    organizational_units
        .iter()
        .find_map(|unit| match unit.as_str() {
            "Solar Inverter" => Some("solar_inverter_device"),
            "Gen3 Wall Connector" => Some("wall_connector_device"),
            _ => None,
        })
}

const KNOWN_OID_ISSUERS: &[&str] = &[
    "Tesla Motors Product Issuing CA",
    "Tesla Motors Product RSA Issuing CA",
    "Tesla Motors GF3 Product Issuing CA",
    "Tesla Motors GF3 Product RSA Issuing CA",
    "Tesla Energy Product Issuing CA",
    "Tesla Energy GF0 Product Issuing CA",
    "Tesla Motors GF0 Product Issuing CA",
    "Tesla Motors GF Austin Product Issuing CA",
    "Tesla Motors GF Berlin Product Issuing CA",
    "Tesla Product Access Issuing CA",
    "Tesla China Product Access Issuing CA",
    "Tesla GF0 Product Access Issuing CA",
    "Tesla GF3 Product Access Issuing CA",
    "Tesla GF Austin Product Access Issuing CA",
    "Tesla GF Berlin Product Access Issuing CA",
];

fn common_name(name: &X509Name<'_>) -> Option<String> {
    let cn = name.iter_common_name().next()?;
    cn.as_str().ok().map(str::to_owned)
}

fn organizational_units(name: &X509Name<'_>) -> Vec<String> {
    name.iter_organizational_unit()
        .filter_map(|unit| unit.as_str().ok().map(str::to_owned))
        .collect()
}

fn extended_key_usage_oids(cert: &X509Certificate<'_>) -> Result<Vec<String>, TelemetryError> {
    let Some(extension) = cert
        .extended_key_usage()
        .map_err(|_| TelemetryError::Certificate)?
    else {
        return Ok(Vec::new());
    };
    Ok(extension
        .value
        .other
        .iter()
        .map(|oid| oid.to_id_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::device_type_for;

    #[test]
    fn issuer_common_name_selects_vehicle_device() {
        let device = device_type_for("TeslaMotors", &[], &[]).unwrap();
        assert_eq!(device, "vehicle_device");
    }

    #[test]
    fn products_ca_serial_number_ou_is_a_board() {
        let device = device_type_for(
            "Tesla Motors Products CA",
            &["Tesla Motors SN".to_owned()],
            &[],
        )
        .unwrap();
        assert_eq!(device, "vehicle_board_device");
    }

    #[test]
    fn product_issuing_ca_uses_the_eku_oid() {
        let device = device_type_for(
            "Tesla Motors Product Issuing CA",
            &[],
            &["1.3.6.1.4.1.49279.2.5.2".to_owned()],
        )
        .unwrap();
        assert_eq!(device, "energy_device");
    }

    #[test]
    fn organizational_unit_overrides_the_oid() {
        let device = device_type_for(
            "Tesla Motors Product Issuing CA",
            &["Gen3 Wall Connector".to_owned()],
            &["1.3.6.1.4.1.49279.2.5.1".to_owned()],
        )
        .unwrap();
        assert_eq!(device, "wall_connector_device");
    }

    #[test]
    fn wifi_eap_oid_is_excluded() {
        let err = device_type_for(
            "Tesla Motors Product Issuing CA",
            &[],
            &["1.3.6.1.4.1.49279.2.5.1.1".to_owned()],
        )
        .unwrap_err();
        assert!(matches!(err, crate::TelemetryError::ExcludedCertificate));
    }
}
