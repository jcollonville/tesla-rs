// SPDX-License-Identifier: MIT OR Apache-2.0

//! Compiles the vendored Tesla protocol buffers with `prost`.
#![allow(missing_docs)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.include_file("wire.rs");
    config.compile_protos(
        &[
            "proto/universal_message.proto",
            "proto/signatures.proto",
            "proto/vcsec.proto",
            "proto/car_server.proto",
            "proto/errors.proto",
            "proto/keys.proto",
            "proto/common.proto",
            "proto/vehicle.proto",
            "proto/managed_charging.proto",
        ],
        &["proto"],
    )?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
