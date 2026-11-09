//! Config JSON covered by `tesla-fleet-api`.

use tesla_fleet_telemetry::{DeliveryPolicy, FieldConfig, FleetTelemetryConfig};

#[test]
fn config_json_round_trips_through_tesla_fleet_api() {
    let config = FleetTelemetryConfig::builder(
        "telemetry.example.com",
        443,
        "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
    )
    .field_with(
        "Location",
        FieldConfig {
            interval_seconds: 10,
            minimum_delta: Some(50.0),
            resend_interval_seconds: Some(300),
            include_fields: vec!["GpsState".to_owned()],
        },
    )
    .field("Soc", 60)
    .alert_types(["service"])
    .expires_at(1_900_000_000)
    .delivery_policy(DeliveryPolicy::Latest)
    .build()
    .unwrap();
    let value = config.to_value().unwrap();
    let parsed: tesla_fleet_api::TelemetryConfig = serde_json::from_value(value.clone()).unwrap();
    let round = serde_json::to_value(&parsed).unwrap();
    assert_eq!(value, round);
    assert_eq!(parsed.delivery_policy.as_deref(), Some("latest"));
    assert_eq!(
        parsed.fields["Location"].include_fields,
        vec!["GpsState".to_owned()]
    );
    let body = config.request_body(["5YJ3E1EA7KF000001"]).unwrap();
    assert_eq!(body["vins"][0], "5YJ3E1EA7KF000001");
}

#[test]
fn unknown_field_names_are_rejected_unless_allowed() {
    let error = FleetTelemetryConfig::builder("telemetry.example.com", 443, "ca")
        .field("NotAField", 10)
        .build()
        .unwrap_err();
    assert!(error.to_string().contains("Field enum"));
    FleetTelemetryConfig::builder("telemetry.example.com", 443, "ca")
        .allow_unknown_fields()
        .field("NotAField", 10)
        .build()
        .unwrap();
}
