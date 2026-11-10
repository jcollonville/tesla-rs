// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Self-hosted archive of Tesla drives, charges, and vehicle state.
//!
//! This program is a clean-room implementation. It is not TeslaMate and it is not
//! affiliated with Tesla, Inc. TeslaMate is only a behavioural reference for what
//! counts as a drive or a charge.

#![forbid(unsafe_code)]

pub mod cli;

mod app;
mod config;
mod crypto;
mod debug;
mod error;
mod geocode;
mod geofence;
mod http;
mod import;
mod ingest;
mod logs;
mod mqtt;
mod owner_stream;
mod persist;
mod rdp;
mod state;
mod telemetry;
mod units;

#[cfg(test)]
mod itests;
#[cfg(test)]
mod test_util;
