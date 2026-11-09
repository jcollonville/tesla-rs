// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::{Duration, SystemTime};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use secrecy::ExposeSecret;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tesla_auth::Region;
use url::Url;

use crate::error::FleetError;
use crate::models::{
    ChargingHistory, ChargingHistoryQuery, FleetStatus, FleetTelemetryConfig,
    FleetTelemetryConfigRequest, FleetTelemetryConfigUpdate, ListVehiclesQuery, PageRef,
    PartnerAccount, PartnerPublicKey, RegisterPartnerAccount, TelemetryConfig, UserRegion, Vehicle,
    VehicleData, VehicleDataEndpoint, VehicleList, VehicleTag,
};
use crate::policy::{CallClass, RequestPolicy, parse_rate_limit};
use crate::token::AccessTokenProvider;

const USER_AGENT: &str = concat!(
    "tesla-fleet-api/",
    env!("CARGO_PKG_VERSION"),
    " (unofficial)"
);

/// Async Tesla Fleet API client.
///
/// The client never calls `wake_up` unless [`Self::wake_up`] is used. `vehicle_data` is a live,
/// billable device-data call and returns [`FleetError::DeviceUnavailable`] when the vehicle is
/// asleep or offline. Prefer `fleet_status` and `vehicle_data_cached` before paying for a live read.
///
/// Requests use `rustls`. Redirects are disabled. The `Authorization` header is marked sensitive
/// and this crate does not log headers or bodies.
#[derive(Clone)]
pub struct FleetClient<T> {
    http: reqwest::Client,
    base_url: Url,
    tokens: T,
    policy: RequestPolicy,
}

impl<T> std::fmt::Debug for FleetClient<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FleetClient")
            .field("base_url", &self.base_url)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<T: AccessTokenProvider> FleetClient<T> {
    /// Client for a region's documented Fleet API base URL.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when the built-in URL cannot be used, which does not
    /// happen for [`Region`]'s documented URLs, or [`FleetError::Http`] when the HTTP client cannot
    /// be built.
    pub fn for_region(region: Region, tokens: T) -> Result<Self, FleetError> {
        Self::builder(region.fleet_api_base(), tokens)?.build()
    }

    /// Starts a builder for `base_url`.
    ///
    /// HTTPS is required. HTTP is accepted only for loopback, so tests can use a local mock and a
    /// token is not sent in cleartext to a remote host.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when `base_url` is not an acceptable origin.
    pub fn builder(base_url: &str, tokens: T) -> Result<FleetClientBuilder<T>, FleetError> {
        Ok(FleetClientBuilder {
            base_url: normalize_base(base_url)?,
            tokens,
            http: None,
            policy: RequestPolicy::default(),
        })
    }

    /// Base URL used for subsequent calls, including the trailing slash.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// `GET /api/1/users/region`.
    ///
    /// The response is based on the token subject. A token audience is chosen when the token is
    /// minted, so a mismatch means the caller must obtain a token for [`UserRegion::known_region`]
    /// rather than reusing an authorization code.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn user_region(&self) -> Result<UserRegion, FleetError> {
        let decoded = self
            .send(
                reqwest::Method::GET,
                "api/1/users/region",
                &[],
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `GET /api/1/vehicles`.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn list_vehicles(
        &self,
        query: &ListVehiclesQuery,
    ) -> Result<VehicleList, FleetError> {
        let mut pairs = Vec::new();
        if let Some(page) = query.page {
            pairs.push(("page", page.to_string()));
        }
        if let Some(per_page) = query.per_page {
            pairs.push(("per_page", per_page.to_string()));
        }
        let query_ref: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (*k, v.clone())).collect();
        let decoded = self
            .send::<Vec<Vehicle>>(
                reqwest::Method::GET,
                "api/1/vehicles",
                &query_ref,
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(VehicleList {
            vehicles: decoded.value,
            pagination: decoded.pagination,
            count: decoded.count,
        })
    }

    /// Follows numeric `pagination.next` cursors.
    ///
    /// Stops when `next` is absent, does not advance, or `max_pages` is reached. This does not wake
    /// vehicles.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when `per_page` or `max_pages` is zero, or a Fleet API error.
    pub async fn list_all_vehicles(
        &self,
        per_page: u32,
        max_pages: u32,
    ) -> Result<Vec<Vehicle>, FleetError> {
        if per_page == 0 || max_pages == 0 {
            return Err(FleetError::InvalidRequest(
                "per_page and max_pages must be non-zero",
            ));
        }
        let mut page = 1_u32;
        let mut vehicles = Vec::new();
        while page <= max_pages {
            let listed = self
                .list_vehicles(&ListVehiclesQuery {
                    page: Some(page),
                    per_page: Some(per_page),
                })
                .await?;
            vehicles.extend(listed.vehicles);
            let Some(next) = listed
                .pagination
                .as_ref()
                .and_then(|item| item.next.as_ref())
            else {
                break;
            };
            let next_page = match next {
                PageRef::Number(number) => u32::try_from(*number).ok(),
                PageRef::Text(text) => text.parse().ok(),
            };
            let Some(next_page) = next_page else {
                break;
            };
            if next_page <= page {
                break;
            }
            page = next_page;
        }
        Ok(vehicles)
    }

    /// `GET /api/1/vehicles/{vehicle_tag}`.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn vehicle(&self, tag: &VehicleTag) -> Result<Vehicle, FleetError> {
        let path = format!("api/1/vehicles/{tag}");
        let decoded = self
            .send(
                reqwest::Method::GET,
                &path,
                &[],
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `GET /api/1/vehicles/{vehicle_tag}/vehicle_data`.
    ///
    /// This is a live call. It is billed as device data and can wake the vehicle on Tesla's side.
    /// This method does not call [`Self::wake_up`]. Pass an explicit `endpoints` list; an empty
    /// list omits the query and lets Tesla choose its default set, which is larger and more expensive.
    /// `location_data` is required on firmware 2023.38+ to receive location, and it shows a location
    /// icon in the vehicle. Location is omitted for a share with `granular_access.hide_private`.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::DeviceUnavailable`] when the vehicle is asleep, offline, or slow.
    pub async fn vehicle_data(
        &self,
        tag: &VehicleTag,
        endpoints: &[VehicleDataEndpoint],
    ) -> Result<VehicleData, FleetError> {
        self.vehicle_data_at("vehicle_data", tag, endpoints).await
    }

    /// `GET /api/1/vehicles/{vehicle_tag}/vehicle_data_cached`.
    ///
    /// Returns the last data the vehicle reported to Tesla. It works while the vehicle is asleep
    /// and does not wake it. The payload is a subset of [`VehicleData`] and can be hours old; use
    /// each block's `timestamp`. Tesla returns 404 when no cache exists and 412 when the vehicle
    /// does not support cached data.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::NotFound`] or [`FleetError::PreconditionFailed`] for those cases.
    pub async fn vehicle_data_cached(&self, tag: &VehicleTag) -> Result<VehicleData, FleetError> {
        self.vehicle_data_at("vehicle_data_cached", tag, &[]).await
    }

    async fn vehicle_data_at(
        &self,
        suffix: &str,
        tag: &VehicleTag,
        endpoints: &[VehicleDataEndpoint],
    ) -> Result<VehicleData, FleetError> {
        let path = format!("api/1/vehicles/{tag}/{suffix}");
        let joined = VehicleDataEndpoint::join(endpoints);
        let query = if joined.is_empty() {
            Vec::new()
        } else {
            vec![("endpoints".to_owned(), joined)]
        };
        let query_ref: Vec<(&str, String)> = query
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let decoded = self
            .send(
                reqwest::Method::GET,
                &path,
                &query_ref,
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `POST /api/1/vehicles/{vehicle_tag}/wake_up`.
    ///
    /// Wakes are billed and limited to three per minute per vehicle. The call returns when Tesla
    /// accepts it; the vehicle can take longer to become `online`. Poll [`Self::vehicle`] rather
    /// than calling `vehicle_data` immediately. This method does not poll.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn wake_up(&self, tag: &VehicleTag) -> Result<Vehicle, FleetError> {
        let path = format!("api/1/vehicles/{tag}/wake_up");
        let decoded = self
            .send(reqwest::Method::POST, &path, &[], None, CallClass::Mutation)
            .await?;
        Ok(decoded.value)
    }

    /// `POST /api/1/vehicles/fleet_status`.
    ///
    /// Use this before `vehicle_data` or `wake_up`. It reports command-protocol readiness and, for
    /// a single VIN, whether device data is discounted. It does not wake the vehicle.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when `vins` is empty or a VIN is not a valid tag.
    pub async fn fleet_status(&self, vins: &[String]) -> Result<FleetStatus, FleetError> {
        validate_vins(vins)?;
        let body = serde_json::json!({ "vins": vins });
        let decoded = self
            .send(
                reqwest::Method::POST,
                "api/1/vehicles/fleet_status",
                &[],
                Some(body),
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `POST /api/1/vehicles/fleet_telemetry_config`.
    ///
    /// Tesla recommends sending this through the Vehicle Command proxy so the configuration is
    /// signed. Legacy applications may call it directly. This crate does not sign the body; that
    /// belongs to `tesla-vehicle-command`. The POST has a concurrency limit of one and returns 429
    /// when overlapped. A 403 with location fields means the share hid private location.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when the config is locally invalid.
    pub async fn create_fleet_telemetry_config(
        &self,
        request: &FleetTelemetryConfigRequest,
    ) -> Result<FleetTelemetryConfigUpdate, FleetError> {
        validate_telemetry_request(request)?;
        let body = to_json(request)?;
        let decoded = self
            .send(
                reqwest::Method::POST,
                "api/1/vehicles/fleet_telemetry_config",
                &[],
                Some(body),
                CallClass::Mutation,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `GET /api/1/vehicles/{vehicle_tag}/fleet_telemetry_config`.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn fleet_telemetry_config(
        &self,
        tag: &VehicleTag,
    ) -> Result<FleetTelemetryConfig, FleetError> {
        let path = format!("api/1/vehicles/{tag}/fleet_telemetry_config");
        let decoded = self
            .send(
                reqwest::Method::GET,
                &path,
                &[],
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `DELETE /api/1/vehicles/{vehicle_tag}/fleet_telemetry_config`.
    ///
    /// A partner token removes the configuration for any vehicle. A user token removes the
    /// calling application's configuration.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn delete_fleet_telemetry_config(
        &self,
        tag: &VehicleTag,
    ) -> Result<FleetTelemetryConfigUpdate, FleetError> {
        let path = format!("api/1/vehicles/{tag}/fleet_telemetry_config");
        let decoded = self
            .send(
                reqwest::Method::DELETE,
                &path,
                &[],
                None,
                CallClass::Mutation,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `GET /api/1/dx/charging/history`.
    ///
    /// Requires the `vehicle_charging_cmds` scope. This does not wake the vehicle.
    ///
    /// # Errors
    ///
    /// Returns the Fleet API status mapped by [`FleetError`].
    pub async fn charging_history(
        &self,
        query: &ChargingHistoryQuery,
    ) -> Result<ChargingHistory, FleetError> {
        let mut pairs = Vec::new();
        if let Some(vin) = query.vin.as_deref() {
            pairs.push(("vin", vin.to_owned()));
        }
        if let Some(start) = query.start_time.as_deref() {
            pairs.push(("startTime", start.to_owned()));
        }
        if let Some(end) = query.end_time.as_deref() {
            pairs.push(("endTime", end.to_owned()));
        }
        if let Some(page) = query.page_no {
            pairs.push(("pageNo", page.to_string()));
        }
        if let Some(size) = query.page_size {
            pairs.push(("pageSize", size.to_string()));
        }
        if let Some(sort) = query.sort_by.as_deref() {
            pairs.push(("sortBy", sort.to_owned()));
        }
        if let Some(order) = query.sort_order {
            pairs.push(("sortOrder", order.as_str().to_owned()));
        }
        let query_ref: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (*k, v.clone())).collect();
        let decoded = self
            .send(
                reqwest::Method::GET,
                "api/1/dx/charging/history",
                &query_ref,
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `POST /api/1/partner_accounts`.
    ///
    /// Requires a partner token. The PEM public key must already be hosted at
    /// `https://{domain}/.well-known/appspecific/com.tesla.3p.public-key.pem`.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when `domain` is not a lowercase hostname.
    pub async fn register_partner_account(
        &self,
        request: &RegisterPartnerAccount,
    ) -> Result<PartnerAccount, FleetError> {
        validate_domain(&request.domain)?;
        let body = to_json(request)?;
        let decoded = self
            .send(
                reqwest::Method::POST,
                "api/1/partner_accounts",
                &[],
                Some(body),
                CallClass::Mutation,
            )
            .await?;
        Ok(decoded.value)
    }

    /// `GET /api/1/partner_accounts/public_key?domain=`.
    ///
    /// Requires a partner token.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::InvalidRequest`] when `domain` is not a lowercase hostname.
    pub async fn partner_public_key(&self, domain: &str) -> Result<PartnerPublicKey, FleetError> {
        validate_domain(domain)?;
        let query = [("domain", domain.to_owned())];
        let decoded = self
            .send(
                reqwest::Method::GET,
                "api/1/partner_accounts/public_key",
                &query,
                None,
                CallClass::Idempotent,
            )
            .await?;
        Ok(decoded.value)
    }

    async fn send<R: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
        class: CallClass,
    ) -> Result<Decoded<R>, FleetError> {
        let body_bytes = match body {
            Some(body) => Some(
                serde_json::to_vec(&body)
                    .map_err(|_| FleetError::InvalidRequest("request json could not be encoded"))?,
            ),
            None => None,
        };
        let url = self
            .base_url
            .join(path)
            .map_err(|_| FleetError::InvalidRequest("invalid request path"))?;
        let mut attempt = 0_u32;
        loop {
            let token = self.tokens.access_token().await?;
            let mut request = self.http.request(method.clone(), url.clone());
            request = authorize(request, token.expose_secret())?;
            request = request.header(ACCEPT, "application/json");
            request = request.header(CONTENT_TYPE, "application/json");
            if !query.is_empty() {
                request = request.query(query);
            }
            if let Some(bytes) = body_bytes.as_deref() {
                request = request.body(bytes.to_vec());
            }
            let response = request.send().await.map_err(FleetError::http)?;
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.bytes().await.map_err(FleetError::http)?;
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let info = parse_rate_limit(&headers, SystemTime::now());
                if should_retry(class, &self.policy, attempt, info.retry_after) {
                    let wait = info.retry_after.unwrap_or(Duration::ZERO);
                    attempt += 1;
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(FleetError::RateLimited(info));
            }
            if !status.is_success() {
                return Err(map_status(status, &bytes));
            }
            return decode_success(&bytes);
        }
    }
}

struct Decoded<T> {
    value: T,
    pagination: Option<crate::models::Pagination>,
    count: Option<i64>,
}

fn should_retry(
    class: CallClass,
    policy: &RequestPolicy,
    attempt: u32,
    retry_after: Option<Duration>,
) -> bool {
    let class_ok = match class {
        CallClass::Idempotent => true,
        CallClass::Mutation => policy.retry_non_idempotent,
    };
    if !class_ok || attempt >= policy.max_rate_limit_retries {
        return false;
    }
    match retry_after {
        Some(wait) => wait <= policy.max_rate_limit_wait,
        None => false,
    }
}

fn authorize(
    request: reqwest::RequestBuilder,
    bearer: &str,
) -> Result<reqwest::RequestBuilder, FleetError> {
    let mut header = HeaderValue::from_str(&format!("Bearer {bearer}"))
        .map_err(|_| FleetError::InvalidRequest("access token is not a valid http header value"))?;
    header.set_sensitive(true);
    Ok(request.header(AUTHORIZATION, header))
}

fn decode_success<T: DeserializeOwned>(bytes: &[u8]) -> Result<Decoded<T>, FleetError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| FleetError::Decode)?;
    let Some(obj) = value.as_object() else {
        return Err(FleetError::Decode);
    };
    let response_null = obj.get("response").is_none_or(serde_json::Value::is_null);
    if response_null && obj.get("error").is_some_and(|item| !item.is_null()) {
        return Err(map_error_value(reqwest::StatusCode::BAD_REQUEST, &value));
    }
    let Some(response) = obj.get("response").cloned() else {
        return Err(FleetError::Decode);
    };
    if response.is_null() {
        return Err(FleetError::Decode);
    }
    let pagination = obj
        .get("pagination")
        .and_then(|item| serde_json::from_value(item.clone()).ok());
    let count = obj.get("count").and_then(serde_json::Value::as_i64);
    let value = serde_json::from_value(response).map_err(|_| FleetError::Decode)?;
    Ok(Decoded {
        value,
        pagination,
        count,
    })
}

fn map_status(status: reqwest::StatusCode, bytes: &[u8]) -> FleetError {
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null);
    map_error_value(status, &value)
}

fn map_error_value(status: reqwest::StatusCode, value: &serde_json::Value) -> FleetError {
    let error = json_string(value.get("error"));
    let description = json_string(value.get("error_description"));
    match status.as_u16() {
        401 => FleetError::Unauthorized { error, description },
        402 => FleetError::PaymentRequired,
        403 => FleetError::Forbidden { error, description },
        404 => FleetError::NotFound { error, description },
        408 => FleetError::DeviceUnavailable { error, description },
        412 => FleetError::PreconditionFailed { error, description },
        421 => FleetError::IncorrectRegion { error, description },
        422 => FleetError::InvalidResource { error, description },
        423 => FleetError::Locked,
        451 => FleetError::UnavailableForLegalReasons { description },
        _ => FleetError::Api {
            status: status.as_u16(),
            error,
            description,
        },
    }
}

fn json_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn to_json(body: &impl Serialize) -> Result<serde_json::Value, FleetError> {
    serde_json::to_value(body)
        .map_err(|_| FleetError::InvalidRequest("request json could not be encoded"))
}

fn validate_vins(vins: &[String]) -> Result<(), FleetError> {
    if vins.is_empty() {
        return Err(FleetError::InvalidRequest("fleet_status requires a vin"));
    }
    for vin in vins {
        VehicleTag::new(vin.clone())?;
    }
    Ok(())
}

fn validate_domain(domain: &str) -> Result<(), FleetError> {
    let ok = !domain.is_empty()
        && domain == domain.to_ascii_lowercase()
        && !domain.contains('/')
        && !domain.contains(' ')
        && !domain.contains(':')
        && domain.contains('.');
    if ok {
        Ok(())
    } else {
        Err(FleetError::InvalidRequest(
            "partner domain must be a lowercase hostname",
        ))
    }
}

fn validate_telemetry_request(request: &FleetTelemetryConfigRequest) -> Result<(), FleetError> {
    validate_vins(&request.vins)?;
    validate_telemetry_config(&request.config)
}

fn validate_telemetry_config(config: &TelemetryConfig) -> Result<(), FleetError> {
    if config.hostname.is_empty()
        || config.hostname.chars().any(char::is_whitespace)
        || config.hostname.contains("://")
        || config.ca.is_empty()
        || config.port == 0
        || config.fields.is_empty()
    {
        return Err(FleetError::InvalidRequest(
            "fleet telemetry config is incomplete",
        ));
    }
    if config
        .fields
        .values()
        .any(|field| field.interval_seconds == 0)
    {
        return Err(FleetError::InvalidRequest(
            "fleet telemetry interval_seconds must be non-zero",
        ));
    }
    Ok(())
}

/// Builder for [`FleetClient`].
#[derive(Debug)]
pub struct FleetClientBuilder<T> {
    base_url: Url,
    tokens: T,
    http: Option<reqwest::Client>,
    policy: RequestPolicy,
}

impl<T> FleetClientBuilder<T> {
    /// Replaces the default rate-limit policy.
    #[must_use]
    pub fn policy(mut self, policy: RequestPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Replaces the default HTTP client.
    #[must_use]
    pub fn http(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::Http`] when the default HTTP client cannot be built.
    pub fn build(self) -> Result<FleetClient<T>, FleetError>
    where
        T: AccessTokenProvider,
    {
        let http = match self.http {
            Some(http) => http,
            None => reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
                .connect_timeout(Duration::from_secs(15))
                .build()
                .map_err(FleetError::http)?,
        };
        Ok(FleetClient {
            http,
            base_url: self.base_url,
            tokens: self.tokens,
            policy: self.policy,
        })
    }
}

fn normalize_base(input: &str) -> Result<Url, FleetError> {
    let mut url = Url::parse(input).map_err(|_| FleetError::InvalidRequest("invalid base url"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FleetError::InvalidRequest(
            "base url must not contain credentials",
        ));
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => {
            return Err(FleetError::InvalidRequest(
                "base url must be https, or http on loopback",
            ));
        }
    }
    let path = url.path().to_owned();
    if !path.ends_with('/') {
        let trimmed = path.trim_end_matches('/');
        url.set_path(&format!("{trimmed}/"));
    }
    Ok(url)
}
