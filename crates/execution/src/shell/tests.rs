use super::*;

#[test]
fn login_overrides_the_profile_default_for_both_powershell_programs() {
    for program in ["powershell.exe", "pwsh.exe"] {
        for (login, use_profile, no_profile) in [
            (None, false, true),
            (None, true, false),
            (Some(true), false, false),
            (Some(true), true, false),
            (Some(false), false, true),
            (Some(false), true, true),
        ] {
            let command = exec_shell_command(
                PlatformShell::PowerShell,
                program.into(),
                "echo test",
                login,
                use_profile,
            )
            .unwrap();
            assert_eq!(command[0], program);
            assert_eq!(command.contains(&"-NoProfile".into()), no_profile);
            assert!(command.contains(&"-NonInteractive".into()));
            assert_eq!(
                command.last().unwrap(),
                &format!("{POWERSHELL_UTF8_OUTPUT_PREFIX}echo test")
            );
        }
    }
}

#[test]
fn posix_login_defaults_to_true_independently_of_the_profile_setting() {
    for use_profile in [false, true] {
        for (login, flag) in [(None, "-lc"), (Some(true), "-lc"), (Some(false), "-c")] {
            let command = exec_shell_command(
                PlatformShell::Posix,
                "bash".into(),
                "echo test",
                login,
                use_profile,
            )
            .unwrap();
            assert_eq!(command[1], flag);
        }
    }
}

/// The named shells take the argv of their family, so only the program that
/// was resolved decides which shell runs.
#[test]
fn the_named_shells_keep_their_family_argv() {
    for shell in [PlatformShell::Posix, PlatformShell::Bash, PlatformShell::Sh] {
        let command =
            exec_shell_command(shell, "sh".into(), "echo test", Some(false), false).unwrap();
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c", "{shell:?}");
    }
    for shell in [
        PlatformShell::PowerShell,
        PlatformShell::Pwsh,
        PlatformShell::WindowsPowerShell,
    ] {
        let command =
            exec_shell_command(shell, "pwsh".into(), "echo test", Some(false), false).unwrap();
        assert_eq!(command[0], "pwsh");
        assert!(command.contains(&"-NoProfile".into()), "{shell:?}");
        assert!(command.contains(&"-NonInteractive".into()), "{shell:?}");
    }
}

#[test]
fn a_shell_belongs_to_one_family_only() {
    for shell in [PlatformShell::Posix, PlatformShell::Bash, PlatformShell::Sh] {
        assert!(shell.is_posix(), "{shell:?}");
        assert!(!shell.is_powershell(), "{shell:?}");
    }
    for shell in [
        PlatformShell::PowerShell,
        PlatformShell::Pwsh,
        PlatformShell::WindowsPowerShell,
    ] {
        assert!(shell.is_powershell(), "{shell:?}");
        assert!(!shell.is_posix(), "{shell:?}");
    }
    assert!(!PlatformShell::Cmd.is_posix());
    assert!(!PlatformShell::Cmd.is_powershell());
}
