mod common;

use oxc_react_docgen_tsc_client::{Supervisor, SupervisorConfig, TscClientError};

#[tokio::test]
async fn a_version_not_in_the_pinned_list_is_rejected_before_spawning() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Layout carries a real tsc binary at a real version, but the config pins a
    // different one — check_version must reject it without ever spawning the process
    // (the fake binary would otherwise happily talk to us).
    let (tsc_path, cwd) = common::layout_with_version(&dir, "6.0.0");
    let config = SupervisorConfig::new(tsc_path, cwd, &["7.1.0-dev.20260926.1"]);

    match Supervisor::spawn(config).await {
        Err(TscClientError::UnsupportedVersion { found, expected, .. }) => {
            assert_eq!(found, "6.0.0");
            assert_eq!(expected, ["7.1.0-dev.20260926.1"].as_slice());
        }
        Err(other) => panic!("expected UnsupportedVersion, got {other:?}"),
        Ok(_) => panic!("expected UnsupportedVersion, but spawn succeeded"),
    }
}

#[tokio::test]
async fn an_unreadable_package_json_is_reported_distinctly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let typescript_dir = dir.path().join("node_modules").join("typescript");
    std::fs::create_dir_all(typescript_dir.join("bin")).expect("create dirs");
    // No package.json written at all.
    let tsc_path = camino::Utf8PathBuf::from_path_buf(typescript_dir.join("bin").join("tsc")).expect("utf8");
    let cwd = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8");
    let config = SupervisorConfig::new(tsc_path, cwd, &[common::PINNED_VERSION]);

    match Supervisor::spawn(config).await {
        Err(TscClientError::VersionCheckFailed { .. }) => {}
        Err(other) => panic!("expected VersionCheckFailed, got {other:?}"),
        Ok(_) => panic!("expected VersionCheckFailed, but spawn succeeded"),
    }
}
