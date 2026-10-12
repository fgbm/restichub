//! Generates Rust code from the Protocol Buffers contract (ADR-0008).
//!
//! The build uses the pure-Rust `protox` compiler. It does not need a system
//! `protoc`. The generated code goes to `OUT_DIR` and is not committed.

/// Protocol Buffers files of the contract, relative to the module root.
const PROTO_FILES: &[&str] = &[
    "restichub/agent/v1/control.proto",
    "restichub/agent/v1/enrollment.proto",
    "restichub/agent/v1/job.proto",
    "restichub/agent/v1/results.proto",
    "restichub/agent/v1/steps.proto",
];

/// Messages that carry secrets. The build does not generate `Debug` for them;
/// the crate implements `Debug` and prints "***" for each secret (FR-PR-10,
/// AC-AG-03).
const SECRET_MESSAGES: &[&str] = &[
    ".restichub.agent.v1.SignedJob",
    ".restichub.agent.v1.RepositoryAccess",
    ".restichub.agent.v1.S3Credentials",
    ".restichub.agent.v1.RestCredentials",
    ".restichub.agent.v1.SftpCredentials",
    ".restichub.agent.v1.EnrollRequest",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../proto");

    for file in PROTO_FILES {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }

    let files = PROTO_FILES.iter().map(|file| root.join(file));
    let descriptors = protox::compile(files, [&root])?;

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        // The RPC `Connect` makes a client method `connect`. The transport
        // helper `connect(dst)` then has the same name. The agent builds its own
        // channel, so the helper is not necessary.
        .build_transport(false)
        .skip_debug(SECRET_MESSAGES)
        .compile_fds(descriptors)?;

    Ok(())
}
