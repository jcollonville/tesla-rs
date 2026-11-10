// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Optional MQTT publisher with Home Assistant discovery. Off unless `MQTT_ENABLED` is set.

use std::time::Duration;

use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::{broadcast, watch};

use crate::config::Config;
use crate::debug::DebugBoard;
use crate::error::redact_secrets;
use crate::persist::LiveEvent;

pub(crate) async fn run(
    config: &Config,
    db: PgPool,
    mut events: broadcast::Receiver<LiveEvent>,
    debug: DebugBoard,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.mqtt_enabled {
        debug.set_mqtt("disabled", None);
        return;
    }
    loop {
        if *shutdown.borrow() {
            break;
        }
        debug.set_mqtt("connecting", None);
        match session(config, &db, &mut events, &debug, &mut shutdown).await {
            Ok(()) => break,
            Err(err) => {
                let err = redact_secrets(&err);
                debug.set_mqtt("error", Some(err.clone()));
                tracing::warn!(error = %err, "mqtt session ended");
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            break;
                        }
                    }
                    () = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            }
        }
    }
}

async fn session(
    config: &Config,
    db: &PgPool,
    events: &mut broadcast::Receiver<LiveEvent>,
    debug: &DebugBoard,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let mut options = MqttOptions::new("tesla-logger", config.mqtt_host.as_str(), config.mqtt_port);
    options.set_keep_alive(Duration::from_secs(30));
    if let (Some(username), Some(password)) = (&config.mqtt_username, &config.mqtt_password) {
        options.set_credentials(username, secrecy::ExposeSecret::expose_secret(password));
    }
    let (client, mut eventloop) = AsyncClient::new(options, 32);
    publish_current(&client, config, db).await?;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return Ok(());
                }
            }
            incoming = events.recv() => {
                match incoming {
                    Ok(event) => publish_event(&client, config, &event).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            polled = eventloop.poll() => {
                match polled {
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                        debug.set_mqtt("connected", None);
                    }
                    Ok(_) => {}
                    Err(err) => return Err(err.to_string()),
                }
            }
        }
    }
}

async fn publish_current(client: &AsyncClient, config: &Config, db: &PgPool) -> Result<(), String> {
    let rows = sqlx::query_as::<_, CurrentRow>(
        "SELECT v.vin, v.name, v.model, s.activity, s.battery_level, s.rated_range_km,
                s.odometer_km, s.speed_kmh, s.power_kw, s.charger_power_kw, s.inside_temp_c,
                s.outside_temp_c, s.latitude, s.longitude
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id",
    )
    .fetch_all(db)
    .await
    .map_err(|err| redact_secrets(&err.to_string()))?;
    for row in rows {
        let mut event = LiveEvent::skeleton(row.vin, "offline");
        event.name = row.name;
        event.model = row.model;
        event.battery_level = row.battery_level;
        event.rated_range_km = row.rated_range_km;
        event.odometer_km = row.odometer_km;
        event.speed_kmh = row.speed_kmh;
        event.power_kw = row.power_kw;
        event.charger_power_kw = row.charger_power_kw;
        event.inside_temp_c = row.inside_temp_c;
        event.outside_temp_c = row.outside_temp_c;
        event.latitude = row.latitude;
        event.longitude = row.longitude;
        // Activity text comes from the row when present.
        if let Some(activity) = row.activity {
            // LiveEvent::activity is &'static. Map known values.
            event.activity = crate::state::Activity::parse(&activity).as_str();
        }
        publish_event(client, config, &event).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct CurrentRow {
    vin: String,
    name: Option<String>,
    model: Option<String>,
    activity: Option<String>,
    battery_level: Option<f64>,
    rated_range_km: Option<f64>,
    odometer_km: Option<f64>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
    charger_power_kw: Option<f64>,
    inside_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

async fn publish_event(
    client: &AsyncClient,
    config: &Config,
    event: &LiveEvent,
) -> Result<(), String> {
    let vin = topic_id(&event.vin);
    if vin.is_empty() {
        return Ok(());
    }
    let prefix = topic_id(&config.mqtt_prefix);
    let device = device_payload(&vin, event.name.as_deref(), event.model.as_deref());
    for sensor in sensors(&prefix, &vin, &device) {
        publish(
            client,
            format!(
                "homeassistant/sensor/tesla_logger_{vin}_{}/config",
                sensor.id
            ),
            &sensor.config,
        )
        .await?;
        if let Some(value) = (sensor.value)(event) {
            publish(client, sensor.state_topic, value).await?;
        }
    }
    publish(
        client,
        format!("homeassistant/device_tracker/tesla_logger_{vin}_location/config"),
        &tracker_config(&prefix, &vin, &device),
    )
    .await?;
    if let (Some(latitude), Some(longitude)) = (event.latitude, event.longitude) {
        publish(
            client,
            format!("{prefix}/{vin}/location"),
            json!({ "latitude": latitude, "longitude": longitude }).to_string(),
        )
        .await?;
    }
    Ok(())
}

type ValueFn = fn(&LiveEvent) -> Option<String>;

struct Sensor {
    id: &'static str,
    config: String,
    state_topic: String,
    value: ValueFn,
}

fn sensors(prefix: &str, vin: &str, device: &Value) -> Vec<Sensor> {
    let specs: [(&str, &str, &str, &str, &str, ValueFn); 8] = [
        ("battery", "Battery", "%", "battery", "measurement", battery),
        (
            "range",
            "Rated range",
            "km",
            "distance",
            "measurement",
            range,
        ),
        (
            "odometer",
            "Odometer",
            "km",
            "distance",
            "total_increasing",
            odometer,
        ),
        ("speed", "Speed", "km/h", "speed", "measurement", speed),
        (
            "charger_power",
            "Charger power",
            "kW",
            "power",
            "measurement",
            charger,
        ),
        (
            "inside_temp",
            "Inside temperature",
            "°C",
            "temperature",
            "measurement",
            inside,
        ),
        (
            "outside_temp",
            "Outside temperature",
            "°C",
            "temperature",
            "measurement",
            outside,
        ),
        ("activity", "Activity", "", "", "", activity),
    ];
    specs
        .into_iter()
        .map(|(id, name, unit, class, state_class, value)| {
            let state_topic = format!("{prefix}/{vin}/{id}");
            let mut config = json!({
                "name": name,
                "unique_id": format!("tesla_logger_{vin}_{id}"),
                "state_topic": state_topic,
                "device": device,
            });
            if !unit.is_empty() {
                config["unit_of_measurement"] = json!(unit);
            }
            if !class.is_empty() {
                config["device_class"] = json!(class);
            }
            if !state_class.is_empty() {
                config["state_class"] = json!(state_class);
            }
            Sensor {
                id,
                config: config.to_string(),
                state_topic,
                value,
            }
        })
        .collect()
}

fn tracker_config(prefix: &str, vin: &str, device: &Value) -> String {
    json!({
        "name": "Location",
        "unique_id": format!("tesla_logger_{vin}_location"),
        "json_attributes_topic": format!("{prefix}/{vin}/location"),
        "state_topic": format!("{prefix}/{vin}/location_state"),
        "source_type": "gps",
        "device": device
    })
    .to_string()
}

fn device_payload(vin: &str, name: Option<&str>, model: Option<&str>) -> Value {
    json!({
        "identifiers": [format!("tesla_logger_{vin}")],
        "name": name.unwrap_or(vin),
        "manufacturer": "Tesla",
        "model": model.unwrap_or("Tesla"),
    })
}

fn battery(event: &LiveEvent) -> Option<String> {
    event.battery_level.map(|value| format!("{value}"))
}
fn range(event: &LiveEvent) -> Option<String> {
    event.rated_range_km.map(|value| format!("{value}"))
}
fn odometer(event: &LiveEvent) -> Option<String> {
    event.odometer_km.map(|value| format!("{value}"))
}
fn speed(event: &LiveEvent) -> Option<String> {
    event.speed_kmh.map(|value| format!("{value}"))
}
fn charger(event: &LiveEvent) -> Option<String> {
    event.charger_power_kw.map(|value| format!("{value}"))
}
fn inside(event: &LiveEvent) -> Option<String> {
    event.inside_temp_c.map(|value| format!("{value}"))
}
fn outside(event: &LiveEvent) -> Option<String> {
    event.outside_temp_c.map(|value| format!("{value}"))
}
#[allow(clippy::unnecessary_wraps)]
fn activity(event: &LiveEvent) -> Option<String> {
    Some(event.activity.to_owned())
}

fn topic_id(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect()
}

async fn publish(
    client: &AsyncClient,
    topic: String,
    payload: impl AsRef<[u8]>,
) -> Result<(), String> {
    client
        .publish(topic, QoS::AtLeastOnce, true, payload.as_ref().to_vec())
        .await
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{device_payload, sensors, topic_id};
    use crate::persist::LiveEvent;

    #[test]
    fn discovery_payloads_do_not_include_secrets() {
        let device = device_payload("5YJ3", Some("Red"), Some("modely"));
        let sensors = sensors("tesla_logger", "5YJ3", &device);
        let battery = sensors
            .iter()
            .find(|sensor| sensor.id == "battery")
            .unwrap();
        assert!(battery.config.contains("\"device_class\":\"battery\""));
        assert!(battery.config.contains("tesla_logger/5YJ3/battery"));
        assert!(!battery.config.to_lowercase().contains("password"));
        assert!(!battery.config.to_lowercase().contains("token"));
        assert_eq!(topic_id("ab c/.."), "abc");
        let mut event = LiveEvent::skeleton("5YJ3", "driving");
        event.vehicle_id = 1;
        event.battery_level = Some(70.0);
        assert_eq!((battery.value)(&event).as_deref(), Some("70"));
    }
}
