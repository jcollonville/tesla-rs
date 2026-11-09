//! Decoders checked against bytes produced by `teslamotors/fleet-telemetry`.

use tesla_fleet_telemetry::proto::vehicle_data::Field;
use tesla_fleet_telemetry::{
    DatumValue, DecodeOptions, FieldKey, MessageKind, StreamParts, TypedEnum, decode_alerts,
    decode_connectivity, decode_envelope, decode_errors, decode_vehicle_data, encode_ack,
    encode_format_error, encode_stream,
};

fn options() -> DecodeOptions {
    DecodeOptions {
        vin_override: Some("5YJ3E1EA7KF000001".to_owned()),
        device_client_version: None,
    }
}

#[test]
fn go_envelope_round_trips_and_decodes_vehicle_data() {
    let frame = include_bytes!("fixtures/stream_vehicle_data.bin");
    let envelope = decode_envelope(frame).unwrap();
    assert_eq!(envelope.kind, MessageKind::Stream);
    assert_eq!(envelope.txid, b"txid-1");
    assert_eq!(envelope.topic, b"V");
    assert_eq!(envelope.message_id, b"mid-1");
    assert_eq!(envelope.sender_id, b"vehicle_device.5YJ3E1EA7KF000001");
    assert_eq!(envelope.device_type, b"vehicle_device");
    assert_eq!(envelope.device_id, b"5YJ3E1EA7KF000001");
    assert_eq!(envelope.created_at, 1_700_000_000);
    assert_eq!(envelope.delivered_at_epoch_ms, 1_700_000_000_123);
    assert_eq!(envelope.payload, include_bytes!("fixtures/vehicle_data.pb"));

    let again = encode_stream(&StreamParts {
        txid: &envelope.txid,
        topic: &envelope.topic,
        message_id: &envelope.message_id,
        sender_id: &envelope.sender_id,
        device_type: &envelope.device_type,
        device_id: &envelope.device_id,
        payload: &envelope.payload,
        created_at: envelope.created_at,
        delivered_at_epoch_ms: envelope.delivered_at_epoch_ms,
    })
    .unwrap();
    assert_eq!(again, frame);

    let data = decode_vehicle_data(&envelope.payload, &options()).unwrap();
    assert_eq!(data.vin(), "5YJ3E1EA7KF000001");
    assert!(data.is_resend());
    assert_eq!(data.created_at().unwrap().seconds, 1_700_000_000);
    assert_eq!(data.created_at().unwrap().nanos, 125_000_000);
    let fields: Vec<_> = data.iter().collect();
    assert!(matches!(
        fields.iter().find(|datum| datum.field == FieldKey::Known(Field::Soc)),
        Some(datum) if matches!(datum.value, DatumValue::Double(value) if (value - 81.5).abs() < 1e-9)
    ));
    let location = fields
        .iter()
        .find(|datum| datum.field == FieldKey::Known(Field::Location))
        .unwrap();
    match &location.value {
        DatumValue::Location(value) => {
            assert!((value.latitude - 37.412_374).abs() < 1e-9);
            assert!((value.longitude + 122.145_867).abs() < 1e-9);
        }
        other => panic!("expected location, got {other}"),
    }
    let voltage = fields
        .iter()
        .find(|datum| datum.field == FieldKey::Known(Field::PackVoltage))
        .unwrap();
    assert_eq!(voltage.value.to_string(), "0.00150");
    assert!(matches!(
        fields
            .iter()
            .find(|datum| datum.field == FieldKey::Known(Field::Gear))
            .map(|datum| &datum.value),
        Some(DatumValue::ShiftState(TypedEnum::Known(_)))
    ));
}

#[test]
fn go_ack_and_error_bytes_match() {
    let ack = include_bytes!("fixtures/stream_ack.bin");
    assert_eq!(encode_ack(b"txid-1", b"V").unwrap(), ack);
    let decoded = decode_envelope(ack).unwrap();
    assert_eq!(decoded.kind, MessageKind::Ack);
    assert_eq!(decoded.txid, b"txid-1");
    assert_eq!(decoded.topic, b"V");

    let error = include_bytes!("fixtures/stream_error.bin");
    assert_eq!(encode_format_error(b"txid-1").unwrap(), error);
}

#[test]
fn alerts_errors_and_connectivity_match_the_go_fixtures() {
    let alerts = decode_alerts(include_bytes!("fixtures/vehicle_alerts.pb"), &options()).unwrap();
    assert_eq!(alerts.vin(), "5YJ3E1EA7KF000001");
    let alert = &alerts.alerts()[0];
    assert_eq!(alert.name, "VCFRONT_a999");
    assert_eq!(alert.started_at.unwrap().seconds, 1_700_000_100);
    assert_eq!(alert.ended_at.unwrap().seconds, 1_700_000_200);
    assert_eq!(alert.ended_at.unwrap().nanos, 0);

    let errors = decode_errors(include_bytes!("fixtures/vehicle_errors.pb"), &options()).unwrap();
    assert_eq!(errors.errors()[0].name, "telemetry_config");
    assert_eq!(
        errors.errors()[0].tags.get("reason").map(String::as_str),
        Some("ca")
    );

    let connectivity = decode_connectivity(
        include_bytes!("fixtures/vehicle_connectivity.pb"),
        &options(),
    )
    .unwrap();
    assert_eq!(connectivity.connection_id(), "socket-1");
    assert_eq!(connectivity.network_interface(), "wifi");
    assert!(matches!(
        connectivity.status(),
        TypedEnum::Known(
            tesla_fleet_telemetry::proto::vehicle_connectivity::ConnectivityEvent::Connected
        )
    ));
}

#[test]
fn every_field_number_round_trips() {
    for number in 0..=279 {
        let field = Field::try_from(number).unwrap();
        assert_eq!(i32::from(field), number);
        assert_eq!(Field::from_str_name(field.as_str_name()), Some(field));
    }
    assert!(Field::try_from(280).is_err());
}

#[test]
fn legacy_aeb_client_shifts_the_enum() {
    let data = decode_vehicle_data(
        include_bytes!("fixtures/vehicle_data.pb"),
        &DecodeOptions {
            device_client_version: Some("1.3.0".to_owned()),
            ..options()
        },
    )
    .unwrap();
    let aeb = data
        .iter()
        .find(|datum| datum.field == FieldKey::Known(Field::AutomaticEmergencyBrakingState))
        .unwrap();
    assert!(matches!(
        aeb.value,
        DatumValue::AutomaticEmergencyBrakingState(TypedEnum::Known(
            tesla_fleet_telemetry::proto::vehicle_data::AutomaticEmergencyBrakingStateValue::AutomaticEmergencyBrakingStateUnavailable
        ))
    ));
}

#[test]
fn alerts_stream_envelope_decodes() {
    let frame = include_bytes!("fixtures/stream_alerts.bin");
    let envelope = decode_envelope(frame).unwrap();
    assert_eq!(envelope.topic, b"alerts");
    let again = encode_stream(&StreamParts {
        txid: &envelope.txid,
        topic: &envelope.topic,
        message_id: &envelope.message_id,
        sender_id: &envelope.sender_id,
        device_type: &envelope.device_type,
        device_id: &envelope.device_id,
        payload: &envelope.payload,
        created_at: envelope.created_at,
        delivered_at_epoch_ms: envelope.delivered_at_epoch_ms,
    })
    .unwrap();
    assert_eq!(again, frame);
}
