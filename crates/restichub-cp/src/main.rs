//! ResticHub control plane.

fn main() {
    println!(
        "restichub-cp {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        restichub_proto::PROTOCOL_VERSION
    );
}
