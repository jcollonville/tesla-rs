// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Binary for the tesla-logger archive server.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match tesla_logger::cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
