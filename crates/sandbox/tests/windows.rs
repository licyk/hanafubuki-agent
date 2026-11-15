//! The Windows sandbox end to end: this binary answers the helper's
//! command line first, as the Agent's does, so the commands it confines
//! start it again as the helper. Without a harness, `main` runs each check
//! in turn.

#[cfg(windows)]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
    if std::env::args().nth(1).as_deref() == Some("--test-connect") {
        let address = std::env::args().nth(2).unwrap();
        match std::net::TcpStream::connect(address) {
            Ok(_) => std::process::exit(0),
            Err(error) => {
                eprintln!("TCP connection: {error}");
                std::process::exit(7);
            }
        }
    }
    windows::run();
}

#[cfg(not(windows))]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;
    use std::process::{Command, Output};

    use hanafubuki_agent_sandbox::{
        AgentSandboxMode, AgentWindowsSandbox, Sandbox, SandboxPolicy, SandboxSettings,
        SandboxStatus,
    };
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, READ_CONTROL, WRITE_DAC,
    };

    pub(super) fn run() {
        let detected = Sandbox::detect();
        let off = detected.with_windows_sandbox(None);
        assert!(
            off.wrap(&SandboxPolicy::ReadOnly, cmd("echo")).is_err(),
            "an unset windows.sandbox must refuse"
        );
        let sandbox = detected.with_windows_sandbox(Some(AgentWindowsSandbox::Unelevated));
        assert!(
            matches!(sandbox.status(), SandboxStatus::Available { .. }),
            "{:?}",
            sandbox.status()
        );
        type SandboxCheck = fn(&Sandbox);
        let checks: [(&str, SandboxCheck); 8] = [
            (
                "read_only_cannot_write_the_workspace",
                read_only_cannot_write_the_workspace,
            ),
            (
                "workspace_write_writes_the_workspace_and_temp_and_nothing_outside",
                workspace_write_writes_the_workspace_and_temp_and_nothing_outside,
            ),
            (
                "the_repository_stays_read_only",
                the_repository_stays_read_only,
            ),
            (
                "a_read_only_path_whose_access_cannot_change_does_not_refuse_the_command",
                a_read_only_path_whose_access_cannot_change_does_not_refuse_the_command,
            ),
            (
                "a_writable_root_whose_access_cannot_change_still_runs_its_command",
                a_writable_root_whose_access_cannot_change_still_runs_its_command,
            ),
            (
                "without_the_network_a_command_gets_the_offline_environment",
                without_the_network_a_command_gets_the_offline_environment,
            ),
            ("full_access_is_unrestricted", full_access_is_unrestricted),
            (
                "a_failed_confinement_runs_nothing",
                a_failed_confinement_runs_nothing,
            ),
        ];
        for (name, check) in checks {
            check(&sandbox);
            eprintln!("ok: {name}");
        }
        if std::env::var_os("HANAFUBUKI_TEST_ELEVATED_SANDBOX").is_some() {
            hanafubuki_agent_sandbox::setup_windows_sandbox().unwrap();
            ordinary_user_can_verify_but_not_change_filters();
            // Re-running setup repairs the existing installation and filter keys.
            hanafubuki_agent_sandbox::setup_windows_sandbox().unwrap();
            ordinary_user_can_verify_but_not_change_filters();
            let elevated = detected.with_windows_sandbox(Some(AgentWindowsSandbox::Elevated));
            assert!(
                matches!(elevated.status(), SandboxStatus::Available { .. }),
                "{:?}",
                elevated.status()
            );
            for (name, check) in checks {
                check(&elevated);
                eprintln!("ok: elevated {name}");
            }
            stdio_round_trip(&elevated);
            elevated_network_is_enforced(&elevated);
            prior_roots_are_not_writable(&elevated);
            msys_commands(&elevated);
        }
    }

    /// Directories outside `TEMP`, which workspace-write always allows.
    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
    }

    /// `script` run by `cmd.exe`. Paths in it are written unquoted, since
    /// `cmd` does not read the quotes the command line escapes; the test
    /// directories hold no spaces.
    fn cmd(script: &str) -> Vec<String> {
        let comspec =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
        vec![
            comspec,
            "/D".to_string(),
            "/S".to_string(),
            "/C".to_string(),
            script.to_string(),
        ]
    }

    fn run_confined(sandbox: &Sandbox, policy: &SandboxPolicy, command: Vec<String>) -> Output {
        let command = sandbox.wrap(policy, command).unwrap();
        Command::new(&command[0])
            .args(&command[1..])
            .output()
            .unwrap()
    }

    fn write(sandbox: &Sandbox, policy: &SandboxPolicy, file: &Path) -> Output {
        run_confined(
            sandbox,
            policy,
            cmd(&format!("echo written>{}", file.display())),
        )
    }

    /// Workspace-write over `root` and `extra`, whose own `.git` stays
    /// read-only but is not changeable here.
    fn workspace_write_beside(root: &Path, extra: &Path) -> SandboxPolicy {
        let mut policy = workspace_write(root, false);
        if let SandboxPolicy::WorkspaceWrite { writable_roots, .. } = &mut policy {
            writable_roots.push(extra.to_path_buf());
        }
        policy
    }

    /// Workspace-write over `root`, as a turn's settings make it.
    fn workspace_write(root: &Path, network_access: bool) -> SandboxPolicy {
        SandboxSettings {
            mode: AgentSandboxMode::WorkspaceWrite,
            writable_roots: Vec::new(),
            network_access,
            windows_sandbox: Some(AgentWindowsSandbox::Unelevated),
        }
        .policy(&[root.to_path_buf()])
    }

    fn read_only_cannot_write_the_workspace(sandbox: &Sandbox) {
        let workspace = directory();
        std::fs::write(workspace.path().join("readable"), "content").unwrap();
        let read = run_confined(
            sandbox,
            &SandboxPolicy::ReadOnly,
            cmd(&format!(
                "type {}",
                workspace.path().join("readable").display()
            )),
        );
        assert!(read.status.success(), "{read:?}");
        assert_eq!(read.stdout, b"content");

        let file = workspace.path().join("file");
        let written = write(sandbox, &SandboxPolicy::ReadOnly, &file);
        assert!(!written.status.success(), "{written:?}");
        assert!(!file.exists());
        let null = run_confined(sandbox, &SandboxPolicy::ReadOnly, cmd("echo x>NUL"));
        assert!(null.status.success(), "{null:?}");
    }

    fn workspace_write_writes_the_workspace_and_temp_and_nothing_outside(sandbox: &Sandbox) {
        let workspace = directory();
        let outside = directory();
        let policy = workspace_write(workspace.path(), false);

        let nested = workspace.path().join("nested");
        let inside = run_confined(
            sandbox,
            &policy,
            cmd(&format!(
                "mkdir {} && echo written>{}",
                nested.display(),
                nested.join("file").display()
            )),
        );
        assert!(inside.status.success(), "{inside:?}");
        assert_eq!(
            std::fs::read_to_string(nested.join("file")).unwrap(),
            "written\r\n"
        );

        let escaped = outside.path().join("file");
        let written = write(sandbox, &policy, &escaped);
        assert!(!written.status.success(), "{written:?}");
        assert!(!escaped.exists());

        // Closed first: `cmd`'s redirection is refused a file another
        // process holds open (a sharing violation).
        let temporary = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let in_temp = write(sandbox, &policy, &temporary);
        assert!(in_temp.status.success(), "{in_temp:?}");
    }

    fn the_repository_stays_read_only(sandbox: &Sandbox) {
        let workspace = directory();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        let hook = workspace.path().join(".git").join("hook");
        let written = write(sandbox, &workspace_write(workspace.path(), false), &hook);
        assert!(!written.status.success(), "{written:?}");
        assert!(!hook.exists());
        let removed = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            cmd(&format!(
                "rmdir {}",
                workspace.path().join(".git").display()
            )),
        );
        assert!(workspace.path().join(".git").exists(), "{removed:?}");
    }

    /// A read-only path whose DACL this user cannot change (a `.git` under a
    /// directory owned by someone else) must not refuse the command: the
    /// deny that path would carry is skipped, and the root's grant still
    /// confines writes.
    fn a_read_only_path_whose_access_cannot_change_does_not_refuse_the_command(sandbox: &Sandbox) {
        let workspace = directory();
        let outside = directory();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        let extra = directory();
        let metadata = extra.path().join(".git");
        std::fs::create_dir(&metadata).unwrap();
        make_unchangeable(&metadata);
        let policy = workspace_write_beside(workspace.path(), extra.path());
        let inside = workspace.path().join("ran");
        let output = run_confined(
            sandbox,
            &policy,
            cmd(&format!("echo ran>{}", inside.display())),
        );
        assert!(output.status.success(), "{output:?}");
        assert!(inside.exists());
        let escaped = outside.path().join("f");
        let output = run_confined(
            sandbox,
            &policy,
            cmd(&format!("echo escaped>{}", escaped.display())),
        );
        assert!(!output.status.success(), "{output:?}");
        assert!(!escaped.exists());
        let denied = metadata.join("config");
        let output = write(sandbox, &policy, &denied);
        assert!(!output.status.success(), "{output:?}");
        assert!(!denied.exists());
    }

    fn make_unchangeable(directory: &Path) {
        // OWNER RIGHTS suppresses the owner's implicit WRITE_DAC, including
        // for an elevated CI account. A protected DACL excludes inherited
        // write grants; DELETE only lets TempDir remove this empty fixture.
        let sddl: Vec<u16> = "D:P(D;;WD;;;S-1-1-0)(A;;RC;;;S-1-3-4)(A;;FRFXSD;;;S-1-1-0)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: the SDDL is NUL-terminated and the output is owned below.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(converted, 0, "{}", std::io::Error::last_os_error());
        let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: path and descriptor remain valid for the call.
        let changed = unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        };
        let error = std::io::Error::last_os_error();
        // SAFETY: the conversion allocated this descriptor with LocalAlloc.
        unsafe { LocalFree(descriptor) };
        assert_ne!(changed, 0, "{error}");
        assert!(
            std::fs::OpenOptions::new()
                .access_mode(READ_CONTROL)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(directory)
                .is_ok(),
            "the fixture directory is not readable"
        );
        assert!(
            std::fs::OpenOptions::new()
                .access_mode(READ_CONTROL | WRITE_DAC)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(directory)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied),
            "the fixture must deny WRITE_DAC"
        );
    }

    /// A command whose working directory and writable root both lie in a
    /// directory this user may not change must still run: the grants keep the
    /// DACL they find, and the restricted token still confines writes.
    fn a_writable_root_whose_access_cannot_change_still_runs_its_command(sandbox: &Sandbox) {
        let cwd = directory();
        make_unchangeable(cwd.path());
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![cwd.path().to_path_buf()],
            network_access: false,
        };
        let command = sandbox.wrap(&policy, cmd("ver")).unwrap();
        let output = Command::new(&command[0])
            .args(&command[1..])
            .current_dir(cwd.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let denied = cwd.path().join("denied");
        let output = write(sandbox, &policy, &denied);
        assert!(!output.status.success(), "{output:?}");
        assert!(!denied.exists());
    }

    fn without_the_network_a_command_gets_the_offline_environment(sandbox: &Sandbox) {
        let workspace = directory();
        let echo = cmd("echo %SBX_NONET_ACTIVE%");
        let offline = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            echo.clone(),
        );
        assert_eq!(String::from_utf8_lossy(&offline.stdout).trim(), "1");
        let online = run_confined(sandbox, &workspace_write(workspace.path(), true), echo);
        assert_eq!(
            String::from_utf8_lossy(&online.stdout).trim(),
            "%SBX_NONET_ACTIVE%"
        );
    }

    fn full_access_is_unrestricted(sandbox: &Sandbox) {
        let outside = directory();
        let file = outside.path().join("file");
        let written = write(sandbox, &SandboxPolicy::FullAccess, &file);
        assert!(written.status.success(), "{written:?}");
        assert!(file.exists());
    }

    /// A helper that cannot confine the command (here, a command line it
    /// does not know) exits before the command runs.
    fn a_failed_confinement_runs_nothing(sandbox: &Sandbox) {
        let workspace = directory();
        let marker = workspace.path().join("ran");
        let mut command = sandbox
            .wrap(
                &workspace_write(workspace.path(), false),
                cmd(&format!("echo ran>{}", marker.display())),
            )
            .unwrap();
        command.insert(2, "--writable-everything".to_string());
        let output = Command::new(&command[0])
            .args(&command[1..])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(hanafubuki_agent_sandbox::HELPER_FAILURE_EXIT_CODE),
            "{output:?}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("hanafubuki-agent sandbox:"));
        assert!(!marker.exists());
    }
    fn stdio_round_trip(sandbox: &Sandbox) {
        use std::io::Write;
        use std::process::Stdio;
        let workspace = directory();
        let command = sandbox
            .wrap(
                &workspace_write(workspace.path(), false),
                cmd("set /p VALUE=& call echo %%VALUE%%& echo %HANAFUBUKI_TEST_ENV%& echo diagnostic 1>&2"),
            )
            .unwrap();
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .env("HANAFUBUKI_TEST_ENV", "environment-inherited")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"round-trip\r\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("round-trip"),
            "{result:?}"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("diagnostic"),
            "{result:?}"
        );
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("environment-inherited"),
            "{result:?}"
        );
        eprintln!("ok: elevated stdio_round_trip");
    }

    fn ordinary_user_can_verify_but_not_change_filters() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
        use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
            FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterDeleteByKey0,
            FwpmFilterSetSecurityInfoByKey0,
        };
        use windows_sys::Win32::Security::{
            CheckTokenMembership, CreateRestrictedToken, CreateWellKnownSid,
            ImpersonateLoggedOnUser, RevertToSelf, WinBuiltinAdministratorsSid,
            DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, SID_AND_ATTRIBUTES, TOKEN_DUPLICATE,
            TOKEN_IMPERSONATE, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        #[derive(serde::Deserialize)]
        struct FilterKeys {
            filter_keys: [[u8; 16]; 4],
        }
        let accounts = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            .join("hanafubuki-agent/sandbox/secrets/group-accounts.json");
        let keys: FilterKeys = serde_json::from_slice(&std::fs::read(accounts).unwrap()).unwrap();
        let mut administrators = [0u8; 68];
        let mut size = administrators.len() as u32;
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    WinBuiltinAdministratorsSid,
                    null_mut(),
                    administrators.as_mut_ptr().cast(),
                    &mut size,
                )
            },
            0
        );
        let disable = SID_AND_ATTRIBUTES {
            Sid: administrators.as_mut_ptr().cast(),
            Attributes: 0,
        };
        let mut base = null_mut();
        assert_ne!(
            unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_IMPERSONATE,
                    &mut base,
                )
            },
            0
        );
        let base = unsafe { OwnedHandle::from_raw_handle(base as _) };
        let mut token = null_mut();
        assert_ne!(
            unsafe {
                CreateRestrictedToken(
                    base.as_raw_handle() as HANDLE,
                    DISABLE_MAX_PRIVILEGE,
                    1,
                    &disable,
                    0,
                    null(),
                    0,
                    null(),
                    &mut token,
                )
            },
            0
        );
        let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
        assert_ne!(
            unsafe { ImpersonateLoggedOnUser(token.as_raw_handle() as HANDLE) },
            0
        );
        struct Revert;
        impl Drop for Revert {
            fn drop(&mut self) {
                unsafe { RevertToSelf() };
            }
        }
        let _revert = Revert;
        let mut admin = 1;
        assert_ne!(
            unsafe { CheckTokenMembership(null_mut(), disable.Sid, &mut admin) },
            0
        );
        assert_eq!(
            admin, 0,
            "the readiness check must not have administrator access"
        );
        let status = Sandbox::detect()
            .with_windows_sandbox(Some(AgentWindowsSandbox::Elevated))
            .status();
        assert!(
            matches!(status, SandboxStatus::Available { .. }),
            "ordinary user readiness: {status:?}"
        );

        struct Engine(HANDLE);
        impl Drop for Engine {
            fn drop(&mut self) {
                unsafe { FwpmEngineClose0(self.0) };
            }
        }
        let mut engine = null_mut();
        assert_eq!(
            unsafe { FwpmEngineOpen0(null(), 10, null(), null(), &mut engine) },
            0
        );
        let engine = Engine(engine);
        for key in keys.filter_keys {
            let key = windows_sys::core::GUID::from_u128(u128::from_be_bytes(key));
            assert_eq!(
                unsafe { FwpmFilterDeleteByKey0(engine.0, &key) },
                ERROR_ACCESS_DENIED,
                "ordinary users must not remove a network block"
            );
            assert_eq!(
                unsafe {
                    FwpmFilterSetSecurityInfoByKey0(
                        engine.0,
                        &key,
                        DACL_SECURITY_INFORMATION,
                        null(),
                        null(),
                        null(),
                        null(),
                    )
                },
                ERROR_ACCESS_DENIED,
                "ordinary users must not replace the filter DACL"
            );
        }
        eprintln!(
            "ok: ordinary user verifies filters but cannot delete or change their permissions"
        );
    }

    fn prior_roots_are_not_writable(sandbox: &Sandbox) {
        let first = directory();
        let second = directory();
        let old = first.path().join("file");
        assert!(write(sandbox, &workspace_write(first.path(), false), &old)
            .status
            .success());
        let result = write(sandbox, &workspace_write(second.path(), false), &old);
        assert!(
            !result.status.success(),
            "previous workspace remained writable: {result:?}"
        );
        assert!(!write(sandbox, &SandboxPolicy::ReadOnly, &old)
            .status
            .success());
        let powershell = |policy: &SandboxPolicy| {
            run_confined(
                sandbox,
                policy,
                vec![
                    "powershell.exe".into(),
                    "-NoProfile".into(),
                    "-Command".into(),
                    format!(
                        "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value changed",
                        old.display()
                    ),
                ],
            )
        };
        let allowed = powershell(&workspace_write(first.path(), false));
        assert!(allowed.status.success(), "{allowed:?}");
        let denied = powershell(&workspace_write(second.path(), false));
        assert!(!denied.status.success(), "{denied:?}");
        let denied = powershell(&SandboxPolicy::ReadOnly);
        assert!(!denied.status.success(), "{denied:?}");
        eprintln!("ok: elevated prior_roots_are_not_writable");
    }

    fn msys_commands(sandbox: &Sandbox) {
        let shells = [
            r"C:\Program Files\Git\usr\bin\bash.exe",
            r"C:\msys64\usr\bin\bash.exe",
        ];
        for shell in shells {
            assert!(
                Path::new(shell).is_file(),
                "native MSYS fixture missing: {shell}"
            );
            let workspace = directory();
            let outside = directory();
            let root = workspace.path().to_string_lossy().replace('\\', "/");
            let policy = workspace_write(workspace.path(), false);
            let bash = |script: &str| vec![shell.into(), "-c".into(), script.into()];
            let result = run_confined(sandbox, &policy, bash(&format!(
                "printf hello | cat > '{root}/file'; test \"$(cat '{root}/file')\" = hello && printf msys-ok"
            )));
            assert!(result.status.success(), "{shell}: {result:?}");
            assert_eq!(result.stdout, b"msys-ok");
            let read = run_confined(
                sandbox,
                &SandboxPolicy::ReadOnly,
                bash(&format!("cat '{root}/file'")),
            );
            assert!(read.status.success(), "{shell} read-only startup: {read:?}");
            assert_eq!(read.stdout, b"hello");
            // A native parent must also confine an indirectly launched MSYS child.
            let script = workspace.path().join("indirect.cmd");
            std::fs::write(
                &script,
                format!("@\"{shell}\" -c \"printf again > '{root}/file'\"\r\n"),
            )
            .unwrap();
            let indirect = run_confined(sandbox, &policy, cmd(&script.display().to_string()));
            assert!(indirect.status.success(), "{shell} indirect: {indirect:?}");
            assert_eq!(
                std::fs::read(workspace.path().join("file")).unwrap(),
                b"again"
            );
            let result = run_confined(
                sandbox,
                &workspace_write(outside.path(), false),
                bash(&format!("echo changed > '{root}/file'")),
            );
            assert!(
                !result.status.success(),
                "{shell} retained an old workspace write: {result:?}"
            );
            let result = run_confined(
                sandbox,
                &SandboxPolicy::ReadOnly,
                bash(&format!("echo changed > '{root}/file'")),
            );
            assert!(
                !result.status.success(),
                "{shell} read-only write: {result:?}"
            );
            assert_eq!(
                std::fs::read(workspace.path().join("file")).unwrap(),
                b"again"
            );
            let forbidden = outside
                .path()
                .join("escape")
                .to_string_lossy()
                .replace('\\', "/");
            let result = run_confined(
                sandbox,
                &policy,
                bash(&format!("echo escape > '{forbidden}'")),
            );
            assert!(!result.status.success(), "{shell}: {result:?}");
            assert!(!outside.path().join("escape").exists());
            std::fs::create_dir(workspace.path().join(".git")).unwrap();
            let result = run_confined(
                sandbox,
                &policy,
                bash(&format!("echo hook > '{root}/.git/hook'")),
            );
            assert!(!result.status.success(), "{shell}: {result:?}");
            assert!(!workspace.path().join(".git/hook").exists());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let script = format!(
                "exec 3<>/dev/tcp/127.0.0.1/{}",
                listener.local_addr().unwrap().port()
            );
            let result = run_confined(sandbox, &policy, bash(&script));
            assert!(!result.status.success(), "{shell} offline: {result:?}");
            let result = run_confined(
                sandbox,
                &workspace_write(workspace.path(), true),
                bash(&script),
            );
            assert!(result.status.success(), "{shell} online: {result:?}");
            eprintln!("ok: elevated MSYS pipes, fork, filesystem and network: {shell}");
        }
    }

    fn elevated_network_is_enforced(sandbox: &Sandbox) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let workspace = directory();
        let command = || {
            vec![
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                "--test-connect".into(),
                format!("127.0.0.1:{port}"),
            ]
        };
        let baseline = run_confined(sandbox, &SandboxPolicy::FullAccess, command());
        assert!(
            baseline.status.success(),
            "unconfined network baseline: {baseline:?}"
        );
        let blocked = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            command(),
        );
        assert_eq!(blocked.status.code(), Some(7), "{blocked:?}");
        let allowed = run_confined(sandbox, &workspace_write(workspace.path(), true), command());
        assert!(allowed.status.success(), "{allowed:?}");
        eprintln!("ok: elevated_network_is_enforced");
    }
}
