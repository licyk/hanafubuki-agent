use std::ffi::OsString;
use std::path::PathBuf;

use super::*;

fn cargo() -> InstallKind {
    InstallKind::Cargo {
        root: PathBuf::from("/home/me/.cargo"),
        default_root: true,
    }
}

#[test]
fn the_check_runs_only_with_the_setting_on_the_variable_unset_and_not_from_a_checkout() {
    let off = Some(OsString::from("1"));
    assert!(allowed(true, None, &InstallKind::Release));
    assert!(allowed(true, None, &cargo()));
    // An empty value counts as unset.
    assert!(allowed(true, Some(OsString::new()), &InstallKind::Release));
    assert!(!allowed(false, None, &InstallKind::Release));
    assert!(!allowed(true, off.clone(), &InstallKind::Release));
    assert!(!allowed(true, Some(OsString::from("0")), &cargo()));
    assert!(!allowed(true, None, &InstallKind::Checkout));
    assert!(!allowed(false, off, &InstallKind::Checkout));
}

#[test]
fn the_notice_names_the_command_for_the_installation() {
    assert_eq!(
        action_text(&UpdateAction::Replace),
        "hanafubuki-agent update"
    );
    let cargo = UpdateAction::Cargo {
        command: vec![
            "cargo".to_string(),
            "install".to_string(),
            "--locked".to_string(),
            "hanafubuki-agent-cli".to_string(),
        ],
    };
    assert_eq!(
        action_text(&cargo),
        "cargo install --locked hanafubuki-agent-cli"
    );
    assert_eq!(
        action_text(&UpdateAction::Manual(ManualReason::CargoOnWindows)),
        "cargo install --locked hanafubuki-agent-cli"
    );
    for reason in [
        ManualReason::Checkout,
        ManualReason::Unwritable,
        ManualReason::NoReleaseForPlatform,
    ] {
        assert_eq!(
            action_text(&UpdateAction::Manual(reason)),
            "hanafubuki-agent update --check"
        );
    }
}

mod launch {
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use hanafubuki_agent_tui::TuiEvents;
    use hanafubuki_agent_update::test_support::{FakeRelease, FakeResponse};

    use super::super::cache::UPDATE_CHECK_FILE;
    use super::*;

    const NOW: u64 = 1_763_107_200_000;
    const HOUR_MS: u64 = 60 * 60 * 1000;
    const FULL: &str = "0.3.0 · hanafubuki-agent update";

    /// A release answering 0.4.0, and a launch of 0.2.0 from a released
    /// binary in a temporary directory, which also holds the cache.
    struct Fixture {
        release: FakeRelease,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(cache: Option<String>) -> Self {
            let release = FakeRelease::start();
            let answer = release.release_json("v0.4.0", &[]);
            release.answer_latest(FakeResponse::json(&answer));
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("bin")).unwrap();
            fs::write(dir.path().join("bin/hanafubuki-agent"), "").unwrap();
            if let Some(cache) = cache {
                fs::write(dir.path().join(UPDATE_CHECK_FILE), cache).unwrap();
            }
            Self { release, dir }
        }

        fn start(&self, allowed: bool, client: Option<ReleaseClient>) -> Option<TuiUpdateNotice> {
            let executable = self.dir.path().join("bin/hanafubuki-agent");
            let launch = Launch {
                data_dir: self.dir.path().to_path_buf(),
                installation: Installation::at(&executable).unwrap(),
                current: "0.2.0".parse().unwrap(),
                now_ms: NOW,
            };
            launch.start(
                allowed,
                || client.ok_or(UpdateError::DebugBuild),
                TuiEvents::new().update_notifier(),
            )
        }

        fn client(&self) -> Option<ReleaseClient> {
            Some(self.release.client(None))
        }

        fn cache(&self) -> String {
            cache_text(self.dir.path())
        }

        /// The requests once a started check has had time to make one.
        fn requests_after_a_moment(&self) -> usize {
            std::thread::sleep(Duration::from_millis(300));
            self.release.requests().len()
        }

        /// Waits for the check's answer in the cache: one request.
        fn wait_for_the_check(&self) {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !self.cache().contains("0.4.0") {
                assert!(Instant::now() < deadline, "the check never answered");
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(self.release.requests().len(), 1);
        }
    }

    fn cache_text(dir: &Path) -> String {
        fs::read_to_string(dir.join(UPDATE_CHECK_FILE)).unwrap_or_default()
    }

    fn cached(version: &str, at: u64) -> Option<String> {
        Some(format!(
            r#"{{"latestVersion": "{version}", "checkedAtMs": {at}}}"#
        ))
    }

    fn shown(notice: Option<TuiUpdateNotice>) -> Option<String> {
        notice.map(|notice| format!("{} · {}", notice.version, notice.action))
    }

    /// Off (the setting, the variable, a checkout) or without a client (a
    /// debug build without the test URL): no request and no notice, even
    /// from a due cache that holds a newer release, which stays as it was.
    #[test]
    fn off_or_without_a_client_nothing_is_sent_or_shown() {
        for (allowed, with_client) in [(false, true), (true, false)] {
            let stale = cached("0.3.0", NOW - 30 * HOUR_MS);
            let fixture = Fixture::new(stale.clone());
            let client = fixture.client().filter(|_| with_client);
            assert_eq!(fixture.start(allowed, client), None);
            assert_eq!(fixture.requests_after_a_moment(), 0);
            assert_eq!(Some(fixture.cache()), stale);
        }
    }

    #[test]
    fn a_fresh_cache_shows_its_newer_release_without_a_request() {
        let fixture = Fixture::new(cached("0.3.0", NOW - HOUR_MS));
        let notice = fixture.start(true, fixture.client());
        assert_eq!(shown(notice).as_deref(), Some(FULL));
        assert_eq!(fixture.requests_after_a_moment(), 0);

        for version in ["0.2.0", "0.1.9"] {
            let fixture = Fixture::new(cached(version, NOW - HOUR_MS));
            assert_eq!(fixture.start(true, fixture.client()), None, "{version}");
            assert_eq!(fixture.requests_after_a_moment(), 0);
        }
    }

    /// A stale, missing or future-dated cache: its notice now, and one
    /// request whose answer is kept.
    #[test]
    fn a_due_check_asks_once_and_keeps_the_answer() {
        let fixture = Fixture::new(cached("0.3.0", NOW - 20 * HOUR_MS));
        let notice = fixture.start(true, fixture.client());
        assert_eq!(shown(notice).as_deref(), Some(FULL));
        fixture.wait_for_the_check();

        let fixture = Fixture::new(None);
        assert_eq!(fixture.start(true, fixture.client()), None);
        fixture.wait_for_the_check();

        let fixture = Fixture::new(cached("0.3.0", NOW + HOUR_MS));
        fixture.start(true, fixture.client());
        fixture.wait_for_the_check();
    }

    #[test]
    fn a_cache_that_cannot_be_read_makes_no_request() {
        let fixture = Fixture::new(None);
        fs::create_dir(fixture.dir.path().join(UPDATE_CHECK_FILE)).unwrap();
        assert_eq!(fixture.start(true, fixture.client()), None);
        assert_eq!(fixture.requests_after_a_moment(), 0);
    }
}
