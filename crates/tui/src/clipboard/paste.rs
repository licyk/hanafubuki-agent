// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Reading the clipboard for Ctrl+V (Codex's `clipboard_paste.rs`): image
//! files copied in a file manager first, then an image's pixels, then text;
//! on WSL, where the native clipboard is not Windows', an image is asked of
//! Windows through PowerShell (`Get-Clipboard -Format Image`). And what a
//! pasted path names, quoted, escaped or a `file://` URL, as Codex reads
//! it. Everything after the read (`ReadClipboard`) is tested; the read
//! itself needs a real clipboard.

use std::path::PathBuf;

/// What the clipboard holds, as Ctrl+V reads it.
#[derive(Debug)]
pub(crate) enum ClipboardContent {
    /// An image file copied in a file manager.
    ImageFile(PathBuf),
    /// An image's pixels.
    Pixels(image::RgbaImage),
    /// Text, and no image.
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClipboardError {
    /// There is no clipboard to read (no display server, a terminal over
    /// SSH), or it refused.
    Unavailable(String),
    /// Neither an image nor text.
    Empty,
}

/// How the interface reads the clipboard; the tests give their own.
pub(crate) type ReadClipboard = fn() -> Result<ClipboardContent, ClipboardError>;

/// Reads the system clipboard. On WSL an image Windows holds is asked for
/// when the native clipboard has none.
#[cfg(not(target_os = "android"))]
pub(crate) fn read_clipboard() -> Result<ClipboardContent, ClipboardError> {
    let native = read_native();
    #[cfg(target_os = "linux")]
    {
        let has_image = matches!(
            native,
            Ok(ClipboardContent::ImageFile(_) | ClipboardContent::Pixels(_))
        );
        if !has_image && is_probably_wsl() {
            if let Some(pixels) = read_windows_image() {
                return Ok(ClipboardContent::Pixels(pixels));
            }
        }
    }
    native
}

#[cfg(target_os = "android")]
pub(crate) fn read_clipboard() -> Result<ClipboardContent, ClipboardError> {
    Err(ClipboardError::Unavailable(
        "there is no native clipboard on Android".to_string(),
    ))
}

#[cfg(not(target_os = "android"))]
fn read_native() -> Result<ClipboardContent, ClipboardError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| ClipboardError::Unavailable(error.to_string()))?;
    // A file manager's copy is files, a browser's pixels (Codex).
    let files = clipboard.get().file_list().unwrap_or_default();
    if let Some(file) = files
        .into_iter()
        .find(|file| super::image_file::is_image(file))
    {
        return Ok(ClipboardContent::ImageFile(file));
    }
    match clipboard.get_image() {
        Ok(image) => {
            let (width, height) = (image.width as u32, image.height as u32);
            return image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
                .map(ClipboardContent::Pixels)
                .ok_or_else(|| {
                    ClipboardError::Unavailable("the clipboard's image is malformed".to_string())
                });
        }
        Err(arboard::Error::ContentNotAvailable) => {}
        // Windows says why it cannot give an image it has (Codex).
        Err(error) if cfg!(windows) => return Err(ClipboardError::Unavailable(error.to_string())),
        Err(_) => {}
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Ok(ClipboardContent::Text(text)),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Err(ClipboardError::Empty),
        Err(error) => Err(ClipboardError::Unavailable(error.to_string())),
    }
}

/// The image Windows' clipboard holds, read on WSL: PowerShell saves it to
/// a PNG in Windows' temporary directory, which is read and removed.
#[cfg(target_os = "linux")]
fn read_windows_image() -> Option<image::RgbaImage> {
    for program in POWERSHELL_PROGRAMS {
        // A foreground child waited for, as `/diff`'s `git`.
        let Ok(output) = std::process::Command::new(program)
            .args(["-NoProfile", "-Command", POWERSHELL_SCRIPT])
            .stdin(std::process::Stdio::null())
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let path = convert_windows_path_to_wsl(String::from_utf8_lossy(&output.stdout).trim())?;
        let image = image::open(&path).ok().map(|image| image.to_rgba8());
        let _ = std::fs::remove_file(&path);
        return image;
    }
    None
}

/// The PowerShell programs tried on WSL, in order (Codex's).
#[cfg(target_os = "linux")]
const POWERSHELL_PROGRAMS: [&str; 3] = ["powershell.exe", "pwsh", "powershell"];

/// Saves Windows' clipboard image to a temporary PNG and prints its path,
/// in UTF-8 whichever PowerShell runs it; exits 1 with no image (Codex's).
#[cfg(target_os = "linux")]
const POWERSHELL_SCRIPT: &str = r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $img = Get-Clipboard -Format Image; if ($img -ne $null) { $p=[System.IO.Path]::GetTempFileName(); $p = [System.IO.Path]::ChangeExtension($p,'png'); $img.Save($p,[System.Drawing.Imaging.ImageFormat]::Png); Write-Output $p } else { exit 1 }"#;

/// Whether this is Linux under WSL, where Ctrl+V is the terminal's and
/// Ctrl+Alt+V pastes an image (Codex's test).
pub(crate) fn is_probably_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        let version = std::fs::read_to_string("/proc/version").unwrap_or_default();
        let version = version.to_lowercase();
        version.contains("microsoft")
            || version.contains("wsl")
            || std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::env::var_os("WSL_INTEROP").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// The file a paste names, when it is one path: quoted, shell-escaped, a
/// `file://` URL, or a Windows or UNC path (a Windows drive's path read as
/// WSL's `/mnt/<drive>` one when `wsl`).
pub(crate) fn normalize_pasted_path(pasted: &str, wsl: bool) -> Option<PathBuf> {
    let pasted = pasted.trim();
    let unquoted = pasted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            pasted
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(pasted);
    if let Ok(url) = url::Url::parse(unquoted) {
        if url.scheme() == "file" {
            return url.to_file_path().ok();
        }
    }
    // Before the shell's rules, which read a backslash as an escape.
    if let Some(path) = normalize_windows_path(unquoted, wsl) {
        return Some(path);
    }
    let [word] = <[String; 1]>::try_from(shlex::split(pasted)?).ok()?;
    Some(normalize_windows_path(&word, wsl).unwrap_or_else(|| PathBuf::from(word)))
}

/// A drive (`C:\`, `C:/`) or UNC (`\\server\share`) path.
fn normalize_windows_path(input: &str, wsl: bool) -> Option<PathBuf> {
    let mut chars = input.chars();
    let drive = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.next() == Some(':')
        && matches!(chars.next(), Some('\\' | '/'));
    if !drive && !input.starts_with("\\\\") {
        return None;
    }
    if wsl {
        if let Some(converted) = convert_windows_path_to_wsl(input) {
            return Some(converted);
        }
    }
    Some(PathBuf::from(input))
}

/// A Windows drive's path as WSL mounts it (`C:\a\b` → `/mnt/c/a/b`); none
/// for a UNC path.
pub(crate) fn convert_windows_path_to_wsl(input: &str) -> Option<PathBuf> {
    if input.starts_with("\\\\") {
        return None;
    }
    let drive = input.chars().next()?.to_ascii_lowercase();
    if !drive.is_ascii_lowercase() || input.get(1..2) != Some(":") {
        return None;
    }
    let mut path = PathBuf::from(format!("/mnt/{drive}"));
    for component in input
        .get(2..)?
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
    {
        path.push(component);
    }
    Some(path)
}

#[cfg(test)]
mod tests;
