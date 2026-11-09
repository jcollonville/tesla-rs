//! Compiles the vendored Fleet Telemetry protobufs with `prost`.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let bundled = protoc_bin_vendored::include_path()?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let include = out.join("protoc-include");
    let timestamp = include.join("google/protobuf/timestamp.proto");
    std::fs::create_dir_all(timestamp.parent().expect("parent"))?;
    std::fs::copy(bundled.join("google/protobuf/timestamp.proto"), &timestamp)?;

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.btree_map([".telemetry.vehicle_error.VehicleError.tags"]);
    config.compile_protos(
        &[
            "protos/vehicle_data.proto",
            "protos/vehicle_alert.proto",
            "protos/vehicle_error.proto",
            "protos/vehicle_connectivity.proto",
            "protos/vehicle_metric.proto",
        ],
        &["protos", include.to_str().ok_or("include path")?],
    )?;
    println!("cargo:rerun-if-changed=protos");
    Ok(())
}
