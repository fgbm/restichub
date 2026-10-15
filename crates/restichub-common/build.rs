//! Stamps the build with the version that CI passes in `RESTICHUB_RELEASE_VERSION` and the
//! revision that CI passes in `RESTICHUB_BUILD_VERSION`.

fn read(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={name}");
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn main() {
    let version =
        read("RESTICHUB_RELEASE_VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let revision = read("RESTICHUB_BUILD_VERSION").unwrap_or_else(|| "dev".to_owned());
    println!("cargo:rustc-env=RESTICHUB_FULL_VERSION={version}+{revision}");
}
