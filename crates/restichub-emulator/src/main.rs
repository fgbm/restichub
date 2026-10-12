//! ResticHub agent emulator for load tests (bench S2).

fn main() {
    println!(
        "restichub-emulator {} (protocol {}.{})",
        env!("CARGO_PKG_VERSION"),
        restichub_proto::PROTOCOL_VERSION_MAJOR,
        restichub_proto::PROTOCOL_VERSION_MINOR
    );
}
