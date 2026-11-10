// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/store_tests.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).

use super::would_stay_active;

#[test]
fn old_version_would_stay_active_for_local_or_later_versions() {
    assert!(would_stay_active("local", "1.0.0"));
    assert!(would_stay_active("10.0.0", "9.0.0"));
    assert!(!would_stay_active("1.0.0", "2.0.0"));
}
