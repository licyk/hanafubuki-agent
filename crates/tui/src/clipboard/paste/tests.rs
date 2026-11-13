// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::path::PathBuf;

use super::*;

#[cfg(not(windows))]
#[test]
fn a_file_url_is_its_path() {
    assert_eq!(
        normalize_pasted_path("file:///tmp/example%20one.png", false),
        Some(PathBuf::from("/tmp/example one.png"))
    );
}

#[cfg(windows)]
#[test]
fn a_file_url_is_its_path() {
    assert_eq!(
        normalize_pasted_path("file:///C:/Temp/example%20one.png", false),
        Some(PathBuf::from(r"C:\Temp\example one.png"))
    );
}

#[test]
fn quoted_and_escaped_paths_are_unquoted() {
    for pasted in [
        r"/home/user/My\ File.png",
        "\"/home/user/My File.png\"",
        "'/home/user/My File.png'",
        "  /home/user/My\\ File.png\n",
    ] {
        assert_eq!(
            normalize_pasted_path(pasted, false),
            Some(PathBuf::from("/home/user/My File.png")),
            "{pasted}"
        );
    }
}

#[test]
fn two_words_or_an_unclosed_quote_name_no_path() {
    assert_eq!(
        normalize_pasted_path(r"/home/user/a\ b.png /home/user/c.png", false),
        None
    );
    assert_eq!(normalize_pasted_path("look at this", false), None);
    assert_eq!(normalize_pasted_path("\"/a/b.png", false), None);
}

#[test]
fn windows_paths_stay_whole_and_become_wsl_ones_under_wsl() {
    for pasted in [
        r"C:\Users\Alice\My Pictures\example image.png",
        r#""C:\Users\Alice\My Pictures\example image.png""#,
        r"'C:\Users\Alice\My Pictures\example image.png'",
    ] {
        assert_eq!(
            normalize_pasted_path(pasted, false),
            Some(PathBuf::from(
                r"C:\Users\Alice\My Pictures\example image.png"
            )),
            "{pasted}"
        );
        assert_eq!(
            normalize_pasted_path(pasted, true),
            Some(PathBuf::from(
                "/mnt/c/Users/Alice/My Pictures/example image.png"
            )),
            "{pasted}"
        );
    }
    // A UNC path has no WSL mount.
    assert_eq!(
        normalize_pasted_path(r"\\server\share\folder\file.jpg", true),
        Some(PathBuf::from(r"\\server\share\folder\file.jpg"))
    );
}

#[test]
fn a_windows_drive_path_maps_to_its_wsl_mount() {
    assert_eq!(
        convert_windows_path_to_wsl(r"D:\a\\b/c.png"),
        Some(PathBuf::from("/mnt/d/a/b/c.png"))
    );
    assert_eq!(convert_windows_path_to_wsl(r"\\server\share"), None);
    assert_eq!(convert_windows_path_to_wsl("relative.png"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn the_wsl_route_asks_powershell_for_the_clipboards_image_as_a_png() {
    assert_eq!(POWERSHELL_PROGRAMS[0], "powershell.exe");
    for part in [
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8",
        "Get-Clipboard -Format Image",
        "[System.Drawing.Imaging.ImageFormat]::Png",
        "Write-Output $p",
        "else { exit 1 }",
    ] {
        assert!(POWERSHELL_SCRIPT.contains(part), "{part}");
    }
}
