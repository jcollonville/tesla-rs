//! Fleet API client tests against a local mock.

use tesla_fleet_api::{
    BearerToken, ChargingHistoryQuery, FleetClient, FleetTelemetryConfigRequest, ListVehiclesQuery,
    Region, RegisterPartnerAccount, RequestPolicy, SortOrder, TelemetryConfig, TelemetryField,
    VehicleDataEndpoint, VehicleTag,
};
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> FleetClient<BearerToken> {
    FleetClient::builder(&server.uri(), BearerToken::new("test-token"))
        .unwrap()
        .build()
        .unwrap()
}

fn vehicle_json() -> &'static str {
    r#"{
      "response": [{
        "id": 100021,
        "vehicle_id": 99999,
        "vin": "TEST00000000VIN01",
        "display_name": "Owned",
        "state": "asleep",
        "tokens": ["4f993c5b9e2b937b"],
        "backseat_token": null,
        "in_service": false,
        "id_s": "100021",
        "granular_access": {"hide_private": false, "future_flag": true},
        "future_field": true
      }],
      "pagination": {"previous": null, "next": null, "current": 1, "per_page": 100, "count": 1, "pages": 1},
      "count": 1
    }"#
}

#[tokio::test]
async fn lists_vehicles_and_redacts_streaming_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles"))
        .respond_with(ResponseTemplate::new(200).set_body_string(vehicle_json()))
        .mount(&server)
        .await;
    let client = client(&server);
    let page = client
        .list_vehicles(&ListVehiclesQuery::default())
        .await
        .unwrap();
    let vehicle = &page.vehicles[0];
    assert_eq!(vehicle.vin, "TEST00000000VIN01");
    assert!(vehicle.is_asleep());
    assert!(vehicle.additional.contains_key("future_field"));
    assert_eq!(vehicle.tokens[0].expose(), "4f993c5b9e2b937b");
    let rendered = format!("{vehicle:?}");
    assert!(!rendered.contains("4f993c5b9e2b937b"));
    let header = server.received_requests().await.unwrap()[0].headers["authorization"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(header, "Bearer test-token");
}

#[tokio::test]
async fn vehicle_data_selects_endpoints_and_does_not_wake() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles/TEST00000000VIN01/vehicle_data"))
        .and(query_param("endpoints", "charge_state;climate_state"))
        .respond_with(ResponseTemplate::new(408).set_body_string(
            r#"{"error":"vehicle unavailable","error_description":"vehicle is asleep"}"#,
        ))
        .mount(&server)
        .await;
    let client = client(&server);
    let tag = VehicleTag::new("TEST00000000VIN01").unwrap();
    let error = client
        .vehicle_data(
            &tag,
            &[
                VehicleDataEndpoint::ChargeState,
                VehicleDataEndpoint::ClimateState,
            ],
        )
        .await
        .unwrap_err();
    assert!(error.is_device_unavailable());
    assert!(error.to_string().contains("vehicle is asleep"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].url.path().contains("wake_up"));
}

#[tokio::test]
async fn parses_vehicle_data_and_keeps_unknown_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles/TEST00000000VIN01/vehicle_data_cached"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{
              "response": {
                "id": 100021,
                "vin": "TEST00000000VIN01",
                "state": "asleep",
                "tokens": null,
                "charge_state": {"battery_level": 42, "charging_state": "Disconnected", "future_metric": 7},
                "drive_state": {"latitude": 37.7, "longitude": -122.4, "shift_state": null},
                "closures_state": {"door": "closed"}
              }
            }"#,
        ))
        .mount(&server)
        .await;
    let client = client(&server);
    let tag = VehicleTag::new("TEST00000000VIN01").unwrap();
    let data = client.vehicle_data_cached(&tag).await.unwrap();
    let charge = data.charge_state.unwrap();
    assert_eq!(charge.battery_level, Some(42.0));
    assert_eq!(
        charge
            .additional
            .get("future_metric")
            .and_then(|v| v.as_i64()),
        Some(7)
    );
    assert!(data.vehicle.additional.contains_key("closures_state"));
    assert_eq!(data.vehicle.tokens.len(), 0);
    assert!(data.drive_state.unwrap().shift_state.is_none());
}

#[tokio::test]
async fn rate_limit_is_reported_and_optional_retry_respects_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/users/region"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "7")
                .insert_header("ratelimit-limit", "60")
                .insert_header("ratelimit-remaining", "0"),
        )
        .mount(&server)
        .await;
    let client = client(&server);
    let error = client.user_region().await.unwrap_err();
    assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(7)));
    let rendered = error.to_string();
    assert!(rendered.contains("rate limited"));
    assert!(!rendered.contains("test-token"));

    Mock::given(method("GET"))
        .and(path("/api/1/vehicles/TEST00000000VIN01"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles/TEST00000000VIN01"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"online"}}"#,
            ),
        )
        .mount(&server)
        .await;
    let retrying = FleetClient::builder(&server.uri(), BearerToken::new("test-token"))
        .unwrap()
        .policy(RequestPolicy {
            max_rate_limit_retries: 1,
            max_rate_limit_wait: std::time::Duration::from_secs(2),
            retry_non_idempotent: false,
        })
        .build()
        .unwrap();
    let vehicle = retrying
        .vehicle(&VehicleTag::new("TEST00000000VIN01").unwrap())
        .await
        .unwrap();
    assert!(vehicle.is_online());
}

#[tokio::test]
async fn error_body_that_is_not_the_error_field_is_not_displayed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles"))
        .respond_with(ResponseTemplate::new(500).set_body_string("access_token=SUPERSECRETTOKEN"))
        .mount(&server)
        .await;
    let error = client(&server)
        .list_vehicles(&ListVehiclesQuery::default())
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("SUPERSECRETTOKEN"));
    assert!(matches!(
        error,
        tesla_fleet_api::FleetError::Api { status: 500, .. }
    ));
}

#[tokio::test]
async fn fleet_status_telemetry_charging_and_partner_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/1/vehicles/fleet_status"))
        .and(body_string_contains("TEST00000000VIN01"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"response":{"key_paired_vins":[],"unpaired_vins":["TEST00000000VIN01"],"vehicle_info":{"TEST00000000VIN01":{"firmware_version":"2024.14.30","discounted_device_data":true}}}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/1/vehicles/fleet_telemetry_config"))
        .and(body_string_contains("telemetry.example.com"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"response":{"updated_vehicles":1,"skipped_vehicles":{"missing_key":[],"unsupported_hardware":["TEST00000000VIN02"],"max_configs":[]}}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/1/vehicles/TEST00000000VIN01/fleet_telemetry_config"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"response":{"synced":false,"limit_reached":false,"config":{"hostname":"telemetry.example.com","ca":"cert","port":4443,"fields":{"DriveRail":{"interval_seconds":60}}}}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/api/1/vehicles/TEST00000000VIN01/fleet_telemetry_config",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"response":{"updated_vehicles":1}}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/1/dx/charging/history"))
        .and(query_param("startTime", "2023-07-27T11:43:45-07:00"))
        .and(query_param("sortOrder", "DESC"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"response":{"data":[{"sessionId":123,"vin":"TEST00000000VIN01","fees":[{"feeType":"CHARGING","totalDue":18.4,"rateTier3":null}]}]}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/1/partner_accounts"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"response":{"domain":"example.com","public_key":"04ab"}}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/1/partner_accounts/public_key"))
        .and(query_param("domain", "example.com"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"response":{"public_key":"04ab"}}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/1/users/region"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"response":{"region":"eu","fleet_api_base_url":"https://fleet-api.prd.eu.vn.cloud.tesla.com"}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/1/vehicles/TEST00000000VIN01/wake_up"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"online"}}"#,
            ),
        )
        .mount(&server)
        .await;

    let client = client(&server);
    let status = client
        .fleet_status(&["TEST00000000VIN01".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        status.vehicle_info["TEST00000000VIN01"].discounted_device_data,
        Some(true)
    );
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "DriveRail".to_owned(),
        TelemetryField {
            interval_seconds: 60,
            minimum_delta: None,
            resend_interval_seconds: None,
            include_fields: Vec::new(),
        },
    );
    let update = client
        .create_fleet_telemetry_config(&FleetTelemetryConfigRequest {
            vins: vec!["TEST00000000VIN01".to_owned()],
            config: TelemetryConfig {
                hostname: "telemetry.example.com".to_owned(),
                ca: "cert".to_owned(),
                port: 4443,
                fields,
                alert_types: Vec::new(),
                exp: None,
                delivery_policy: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(update.updated_vehicles, 1);
    assert_eq!(
        update.skipped_vehicles.unsupported_hardware,
        vec!["TEST00000000VIN02".to_owned()]
    );
    let tag = VehicleTag::new("TEST00000000VIN01").unwrap();
    assert!(!client.fleet_telemetry_config(&tag).await.unwrap().synced);
    assert_eq!(
        client
            .delete_fleet_telemetry_config(&tag)
            .await
            .unwrap()
            .updated_vehicles,
        1
    );
    let history = client
        .charging_history(&ChargingHistoryQuery {
            start_time: Some("2023-07-27T11:43:45-07:00".to_owned()),
            sort_order: Some(SortOrder::Desc),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(history.data[0].session_id, Some(123));
    assert_eq!(history.data[0].fees[0].total_due, Some(18.4));
    let account = client
        .register_partner_account(&RegisterPartnerAccount {
            domain: "example.com".to_owned(),
            csr: None,
        })
        .await
        .unwrap();
    assert_eq!(account.domain.as_deref(), Some("example.com"));
    assert_eq!(
        client
            .partner_public_key("example.com")
            .await
            .unwrap()
            .public_key,
        "04ab"
    );
    assert_eq!(
        client.user_region().await.unwrap().known_region(),
        Some(Region::Europe)
    );
    assert!(client.wake_up(&tag).await.unwrap().is_online());

    let register = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|request| request.url.path() == "/api/1/partner_accounts")
        .unwrap();
    let body = String::from_utf8(register.body).unwrap();
    assert_eq!(body, r#"{"domain":"example.com"}"#);
}

#[test]
fn rejects_cleartext_remote_base_urls_and_bad_tags() {
    let error = FleetClient::builder("http://example.com", BearerToken::new("token")).unwrap_err();
    assert!(error.to_string().contains("https"));
    assert!(VehicleTag::new("../wake_up").is_err());
    assert!(FleetClient::for_region(Region::China, BearerToken::new("token")).is_ok());
    assert_eq!(
        Region::China.fleet_api_base(),
        "https://fleet-api.prd.cn.vn.cloud.tesla.cn"
    );
}

#[tokio::test]
async fn incorrect_region_and_payment_required_are_distinct() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/users/region"))
        .respond_with(ResponseTemplate::new(421).set_body_string(r#"{"error":"incorrect_region"}"#))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let api = client(&server);
    assert!(matches!(
        api.user_region().await.unwrap_err(),
        tesla_fleet_api::FleetError::IncorrectRegion { .. }
    ));
    Mock::given(method("GET"))
        .and(path("/api/1/users/region"))
        .respond_with(ResponseTemplate::new(402))
        .mount(&server)
        .await;
    // The first mock is exhausted, but match order may still prefer it. Mounting a new server is simpler.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/1/users/region"))
        .respond_with(ResponseTemplate::new(402))
        .mount(&server)
        .await;
    let error = client(&server).user_region().await.unwrap_err();
    assert!(matches!(
        error,
        tesla_fleet_api::FleetError::PaymentRequired
    ));
}

#[tokio::test]
async fn signed_command_posts_base64_routable_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/1/vehicles/TEST00000000VIN01/signed_command"))
        .and(body_string_contains("\"routable_message\":\"AQID/w==\""))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"response":"b2s="}"#))
        .mount(&server)
        .await;
    let tag = VehicleTag::new("TEST00000000VIN01").unwrap();
    let response = client(&server)
        .signed_command(&tag, &[1, 2, 3, 255])
        .await
        .unwrap();
    assert_eq!(response, b"ok");
    let body = server.received_requests().await.unwrap()[0].body.clone();
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("test-token"));
}
