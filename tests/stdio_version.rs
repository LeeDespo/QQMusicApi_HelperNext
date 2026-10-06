//! The `--version` contract the release smoke relies on: the adapter answers
//! with the component and protocol versions, before reading or creating any
//! host configuration.
use std::process::Command;

#[test]
fn version_flag_answers_without_host_configuration() {
    // Deliberately nothing configured: the release smoke runs the binary with
    // no credential directory in the environment.
    let output = Command::new(env!("CARGO_BIN_EXE_qqmusic-helper-next"))
        .arg("--version")
        .env_remove("QQMUSIC_HELPER_NEXT_DIR")
        .env_remove("QQMUSIC_HELPER_DIR")
        .env_remove("QQMUSIC_HELPER_NEXT_PLATFORM")
        .output()
        .expect("spawn the adapter");
    assert!(output.status.success(), "--version must exit 0");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!(
            "qqmusic-helper-next {} (protocol {})",
            qqmusic_api_helper_next::COMPONENT_VERSION,
            qqmusic_api_helper_next::PROTOCOL_VERSION
        )
    );
    assert!(
        output.stderr.is_empty(),
        "a version query must not log: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
