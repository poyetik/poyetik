//! Tests of the execution modes and of what a box type may run in.

#![forbid(unsafe_code)]

use alloc::string::ToString;

use super::{BoxMode, ExecutionMode};

#[test]
fn an_offline_only_box_runs_in_every_mode_but_live() {
    for (mode, offline_only) in [
        (ExecutionMode::Live, false),
        (ExecutionMode::Offline, true),
        (ExecutionMode::Build, true),
    ] {
        assert!(BoxMode::LiveCapable.runs_in(mode), "live-capable, {mode}");
        assert_eq!(
            BoxMode::OfflineOnly.runs_in(mode),
            offline_only,
            "offline-only, {mode}"
        );
    }
}

#[test]
fn names_each_mode() {
    assert_eq!(ExecutionMode::Live.to_string(), "live", "live");
    assert_eq!(ExecutionMode::Offline.to_string(), "offline", "offline");
    assert_eq!(ExecutionMode::Build.to_string(), "build", "build");
}
