mod common;

use caliper_docgen_tsc_client::{Supervisor, SupervisorConfig, TscClientError};

#[tokio::test]
async fn spawning_a_nonexistent_binary_reports_spawn_failed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (tsc_path, cwd) = common::layout_without_binary(&dir, common::PINNED_VERSION);
    let config = SupervisorConfig::new(tsc_path, cwd, &[common::PINNED_VERSION]);

    match Supervisor::spawn(config).await {
        Err(TscClientError::SpawnFailed { .. }) => {}
        Err(other) => panic!("expected SpawnFailed, got {other:?}"),
        Ok(_) => panic!("expected SpawnFailed, but spawn succeeded"),
    }
}
