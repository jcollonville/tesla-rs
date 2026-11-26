// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Self-hosted archive of Tesla drives, charges, and vehicle state.
//!
//! This program is a clean-room implementation. It is not TeslaMate and it is not
//! affiliated with Tesla, Inc. TeslaMate is only a behavioural reference for what
//! counts as a drive or a charge.

#![forbid(unsafe_code)]

pub mod cli;

mod accounts;
mod app;
mod budget;
mod config;
mod connection;
mod crypto;
mod debug;
mod edits;
mod error;
mod geocode;
mod geofence;
mod http;
mod identity;
mod import;
mod ingest;
mod logs;
mod mqtt;
mod owner_stream;
mod partner;
mod passkeys;
mod persist;
mod position_mode;
mod qr;
mod rdp;
mod settings;
mod state;
mod status;
mod system;
mod telemetry;
mod telemetry_config;
mod telemetry_health;
mod units;

#[cfg(test)]
mod itests;
#[cfg(test)]
mod test_util;
