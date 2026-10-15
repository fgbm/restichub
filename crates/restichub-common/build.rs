//! Stamps the build with the revision that CI passes in `RESTICHUB_BUILD_VERSION`.

fn main() {
    println!("cargo:rerun-if-env-changed=RESTICHUB_BUILD_VERSION");
    let revision = std::env::var("RESTICHUB_BUILD_VERSION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "dev".to_owned());
    println!("cargo:rustc-env=RESTICHUB_BUILD_REVISION={revision}");
}
