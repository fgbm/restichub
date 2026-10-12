//! ResticHub agent.

fn main() {
    println!(
        "restichub-agent {} (protocol {}.{})",
        env!("CARGO_PKG_VERSION"),
        restichub_proto::PROTOCOL_VERSION_MAJOR,
        restichub_proto::PROTOCOL_VERSION_MINOR
    );
}
