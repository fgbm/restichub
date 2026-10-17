#![allow(clippy::expect_used)]

use super::*;

#[test]
fn ac_rs_05_the_default_root_is_outside_the_agent_tree() {
    let agent_root = crate::paths::config_dir()
        .parent()
        .map(Path::to_path_buf)
        .expect("the configuration directory has a parent");
    assert!(!default_root().starts_with(&agent_root));
    assert_eq!(
        default_root().file_name().and_then(|name| name.to_str()),
        Some("ResticHub-Restore")
    );
}
