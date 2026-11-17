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
