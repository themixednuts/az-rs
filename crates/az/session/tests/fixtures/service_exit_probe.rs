use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use az_proto_core::{Endpoint, EndpointKind};
use az_service_supervision::{ServiceEndpointKind, ServiceReadyRecord, SupervisedServiceRole};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let run = required_arg(&args, "--run")
        .parse::<uuid::Uuid>()
        .expect("--run must be a UUID");
    let ready_file = PathBuf::from(required_arg(&args, "--ready-file"));
    let exit_code = required_arg(&args, "--probe-exit-code")
        .parse::<i32>()
        .expect("--probe-exit-code must be an i32");
    let exit_signal = PathBuf::from(required_arg(&args, "--probe-exit-signal"));
    let ready_endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41000");
    let mut ready = ServiceReadyRecord::new(
        "runtime-host",
        SupervisedServiceRole::RuntimeHost,
        run,
        &ready_endpoint,
        Some(std::process::id()),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after the Unix epoch")
            .as_millis(),
    );
    ready.observability_endpoint_kind = Some(ServiceEndpointKind::Tcp);
    ready.observability_endpoint_address = Some("127.0.0.1:41001".to_string());
    assert_eq!(required_arg(&args, "--lifecycle-endpoint-kind"), "tcp");
    ready.lifecycle_endpoint_kind = Some(ServiceEndpointKind::Tcp);
    ready.lifecycle_endpoint_address =
        Some(required_arg(&args, "--lifecycle-endpoint").to_string());

    let parent = ready_file
        .parent()
        .expect("--ready-file must have a parent directory");
    std::fs::create_dir_all(parent).expect("ready-file directory must be creatable");
    let staged = ready_file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(
        &staged,
        toml::to_string_pretty(&ready).expect("ready record must serialize"),
    )
    .expect("staged ready record must be writable");
    std::fs::rename(staged, ready_file).expect("ready record must publish atomically");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !exit_signal.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "exit signal was not published"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::process::exit(exit_code);
}

fn required_arg<'a>(args: &'a [String], flag: &str) -> &'a str {
    args.windows(2).find(|pair| pair[0] == flag).map_or_else(
        || panic!("missing required argument {flag}"),
        |pair| pair[1].as_str(),
    )
}
