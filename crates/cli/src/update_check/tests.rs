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

    use hanafubuki_agent_tui::{TuiEvents, TuiUpdateChoice, TuiUpdatePrompt};
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

        /// A launch with a prompt argument: no prompt.
        fn start(&self, allowed: bool, client: Option<ReleaseClient>) -> Option<TuiUpdateNotice> {
            let no_prompt = None::<fn(TuiUpdatePrompt) -> TuiUpdateChoice>;
            match self.launch(allowed, client, no_prompt) {
                Started::Interface(notice) => notice,
                Started::UpdateNow(_) => panic!("no prompt was put"),
            }
        }

        /// A launch whose prompt, if any, `ask` answers.
        fn launch(
            &self,
            allowed: bool,
            client: Option<ReleaseClient>,
            ask: Option<impl FnOnce(TuiUpdatePrompt) -> TuiUpdateChoice>,
        ) -> Started {
            let launch = Launch {
                data_dir: self.dir.path().to_path_buf(),
                installation: Installation::at(&self.executable()).unwrap(),
                current: "0.2.0".parse().unwrap(),
                now_ms: NOW,
            };
            launch.start(
                allowed,
                || client.ok_or(UpdateError::DebugBuild),
                ask,
                TuiEvents::new().update_notifier(),
            )
        }

        /// A launch with the check on whose prompt, if any, is answered
        /// `choice`: the prompt put, and the notice.
        fn prompted(
            &self,
            choice: TuiUpdateChoice,
        ) -> (Option<TuiUpdatePrompt>, Option<TuiUpdateNotice>) {
            let mut put = None;
            let started = self.launch(
                true,
                self.client(),
                Some(|prompt| {
                    put = Some(prompt);
                    choice
                }),
            );
            match started {
                Started::Interface(notice) => (put, notice),
                Started::UpdateNow(_) => panic!("updating after {put:?}"),
            }
        }

        fn executable(&self) -> PathBuf {
            self.dir.path().join("bin/hanafubuki-agent")
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

    fn cached_with_dismissal(version: &str, dismissed: &str) -> Option<String> {
        Some(format!(
            r#"{{"latestVersion": "{version}", "checkedAtMs": {}, "dismissedVersion": "{dismissed}"}}"#,
            NOW - HOUR_MS
        ))
    }

    /// A newer release in the cache is put to the person, worded for a
    /// released binary; Skip goes on into the interface with the notice
    /// and the cache as it was, and asks again at the next launch.
    #[test]
    fn a_newer_cached_release_is_prompted_and_skip_asks_again_next_time() {
        let fixture = Fixture::new(cached("0.3.0", NOW - HOUR_MS));
        let before = fixture.cache();
        let (put, notice) = fixture.prompted(TuiUpdateChoice::Skip);
        assert_eq!(
            put,
            Some(TuiUpdatePrompt {
                current: "0.2.0".to_string(),
                latest: "0.3.0".to_string(),
                release_notes: "https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0"
                    .to_string(),
                update_now: "Update now (downloads 0.3.0 and replaces this executable)".to_string(),
            })
        );
        assert_eq!(shown(notice).as_deref(), Some(FULL));
        assert_eq!(fixture.cache(), before);
        assert!(fixture.prompted(TuiUpdateChoice::Skip).0.is_some());
    }

    /// An equal release (as after an update: `update` leaves the cache, and
    /// the new build is that release) or an older one: no prompt, no
    /// notice.
    #[test]
    fn a_cached_release_that_is_not_newer_is_not_prompted() {
        for version in ["0.2.0", "0.1.9"] {
            let fixture = Fixture::new(cached(version, NOW - HOUR_MS));
            let (put, notice) = fixture.prompted(TuiUpdateChoice::UpdateNow);
            assert_eq!((put, notice), (None, None), "{version}");
        }
    }

    /// "Skip until next version" keeps the cache's other fields; then that
    /// version is not prompted again, its notice still shown, until a newer
    /// one is cached.
    #[test]
    fn skip_until_next_version_stops_the_prompt_until_a_newer_release() {
        let fixture = Fixture::new(Some(format!(
            r#"{{"latestVersion": "0.3.0", "checkedAtMs": {}, "later": [1]}}"#,
            NOW - HOUR_MS
        )));
        let (put, notice) = fixture.prompted(TuiUpdateChoice::SkipVersion);
        assert!(put.is_some());
        assert_eq!(shown(notice).as_deref(), Some(FULL));
        let cache: serde_json::Value = serde_json::from_str(&fixture.cache()).unwrap();
        assert_eq!(
            cache,
            serde_json::json!({
                "latestVersion": "0.3.0",
                "checkedAtMs": NOW - HOUR_MS,
                "dismissedVersion": "0.3.0",
                "later": [1],
            })
        );

        let (put, notice) = fixture.prompted(TuiUpdateChoice::UpdateNow);
        assert_eq!(put, None);
        assert_eq!(shown(notice).as_deref(), Some(FULL));

        let fixture = Fixture::new(cached_with_dismissal("0.3.1", "0.3.0"));
        let (put, _) = fixture.prompted(TuiUpdateChoice::Skip);
        assert_eq!(put.map(|prompt| prompt.latest).as_deref(), Some("0.3.1"));
    }

    /// Off (the setting, the variable, a checkout), without a client (a
    /// debug build without the test URL) or with a prompt argument: no
    /// prompt.
    #[test]
    fn no_prompt_when_the_check_is_off_or_a_prompt_argument_was_given() {
        for (allowed, with_client) in [(false, true), (true, false)] {
            let fixture = Fixture::new(cached("0.3.0", NOW - HOUR_MS));
            let client = fixture.client().filter(|_| with_client);
            let started = fixture.launch(allowed, client, Some(|_| panic!("prompted")));
            assert!(matches!(started, Started::Interface(None)));
        }
        let fixture = Fixture::new(cached("0.3.0", NOW - HOUR_MS));
        // `start` puts no prompt: `ask` is `None`.
        assert_eq!(
            shown(fixture.start(true, fixture.client())).as_deref(),
            Some(FULL)
        );
    }

    /// "Update now" hands the installation back before anything else: no
    /// check starts, even when one is due, and the cache is left as it was.
    #[test]
    fn update_now_starts_no_check() {
        let stale = cached("0.3.0", NOW - 30 * HOUR_MS);
        let fixture = Fixture::new(stale.clone());
        let started = fixture.launch(true, fixture.client(), Some(|_| TuiUpdateChoice::UpdateNow));
        let Started::UpdateNow(installation) = started else {
            panic!("not updating");
        };
        assert_eq!(
            installation.executable().key(),
            Installation::at(&fixture.executable())
                .unwrap()
                .executable()
                .key()
        );
        assert_eq!(fixture.requests_after_a_moment(), 0);
        assert_eq!(Some(fixture.cache()), stale);
    }

    /// The executables an update moved aside are removed at each launch on
    /// Windows, whether the check is on or not; nothing else is, and on Unix
    /// nothing is moved aside, so nothing is removed.
    #[test]
    fn a_launch_removes_the_executables_moved_aside_on_windows() {
        let fixture = Fixture::new(None);
        let bin = fixture.dir.path().join("bin");
        let backup = bin.join("hanafubuki-agent.1763107200000.outdated");
        let other = bin.join("other.1763107200000.outdated");
        fs::write(&backup, "old").unwrap();
        fs::write(&other, "old").unwrap();
        fixture.start(false, None);
        assert_eq!(backup.exists(), cfg!(not(windows)));
        assert!(other.exists());
    }

    #[test]
    fn a_cache_that_cannot_be_read_makes_no_request() {
        let fixture = Fixture::new(None);
        fs::create_dir(fixture.dir.path().join(UPDATE_CHECK_FILE)).unwrap();
        assert_eq!(fixture.start(true, fixture.client()), None);
        assert_eq!(fixture.requests_after_a_moment(), 0);
    }
}
