// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A local file link's target as the transcript shows it: the path with
//! forward slashes, relative to the session's directory when it is under
//! it, and its location (`#L12C3`, `:12:3`, a range) as `:line:column`.
//!
//! This is text, not the filesystem: nothing is resolved or checked, and the
//! path's convention (POSIX, a Windows drive, UNC) is read from the text
//! itself, never from the host the interface runs on.

use std::borrow::Cow;

use url::Url;

/// Whether `destination` names a local path: a `file://` URL, an absolute
/// or `~/`, `./`, `../` path, a Windows drive path or a UNC path.
pub(super) fn is_local_path_like_link(destination: &str) -> bool {
    destination.starts_with("file://")
        || destination.starts_with('/')
        || destination.starts_with("~/")
        || destination.starts_with("./")
        || destination.starts_with("../")
        || destination.starts_with("\\\\")
        || matches!(
            destination.as_bytes(),
            [drive, b':', separator, ..]
                if drive.is_ascii_alphabetic() && matches!(separator, b'/' | b'\\')
        )
}

/// The target of a local link as shown, relative to `cwd` when under it;
/// `None` when a `file://` URL cannot be read.
pub(super) fn render_local_link_target(destination: &str, cwd: Option<&str>) -> Option<String> {
    let (path_text, location_suffix) = parse_local_link_target(destination)?;
    let mut rendered = display_local_link_path(&path_text, cwd);
    if let Some(location_suffix) = location_suffix {
        rendered.push_str(&location_suffix);
    }
    Some(rendered)
}

/// The destination's path (forward slashes, decoded) and its location
/// suffix as `:line[:column][-line[:column]]`.
fn parse_local_link_target(destination: &str) -> Option<(String, Option<String>)> {
    if destination.starts_with("file://") {
        let url = Url::parse(destination).ok()?;
        let path_text = normalize_local_link_path_text(&file_url_path(&url));
        let location_suffix = url.fragment().and_then(hash_location_suffix);
        return Some((path_text, location_suffix));
    }

    let mut path_text = destination;
    let mut location_suffix = None;
    // `path#L10` is a location even when the path also ends in `:digits`.
    if let Some((candidate_path, fragment)) = destination.rsplit_once('#') {
        if let Some(normalized) = hash_location_suffix(fragment) {
            path_text = candidate_path;
            location_suffix = Some(normalized);
        }
    }
    if location_suffix.is_none() {
        if let Some(suffix) = colon_location_suffix(path_text) {
            path_text = &path_text[..path_text.len() - suffix.len()];
            location_suffix = Some(suffix.to_string());
        }
    }
    Some((
        normalize_local_link_path_text(&percent_decode(path_text)),
        location_suffix,
    ))
}

/// A `file:` URL's path in its own convention: `C:/x` for a drive,
/// `//server/share/x` for a host, `/x` otherwise, decoded.
fn file_url_path(url: &Url) -> String {
    let path = percent_decode(url.path());
    let host = url.host_str().filter(|host| !host.is_empty());
    if let Some(host) = host {
        return format!("//{host}{path}");
    }
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_string(),
        _ => path.into_owned(),
    }
}

/// `L12`, `L12C3` or a range of them, as `:12`, `:12:3`, `:12:3-14:9`;
/// `None` for any other fragment, which stays part of the path.
fn hash_location_suffix(fragment: &str) -> Option<String> {
    let (start, end) = match fragment.split_once('-') {
        Some((start, end)) => (start, Some(end)),
        None => (fragment, None),
    };
    let mut normalized = String::from(":");
    push_hash_location_point(&mut normalized, start)?;
    if let Some(end) = end {
        normalized.push('-');
        push_hash_location_point(&mut normalized, end)?;
    }
    Some(normalized)
}

/// `L<line>` or `L<line>C<column>` written as `<line>` or `<line>:<column>`.
fn push_hash_location_point(out: &mut String, point: &str) -> Option<()> {
    let point = point.strip_prefix('L')?;
    let (line, column) = match point.split_once('C') {
        Some((line, column)) => (line, Some(column)),
        None => (point, None),
    };
    if !is_digits(line) || column.is_some_and(|column| !is_digits(column)) {
        return None;
    }
    out.push_str(line);
    if let Some(column) = column {
        out.push(':');
        out.push_str(column);
    }
    Some(())
}

/// The trailing `:line`, `:line:column` or range (`-` or `–` between two of
/// them) of a path; a drive's colon (`C:/`) is never one.
pub(super) fn colon_location_suffix(path_text: &str) -> Option<&str> {
    path_text
        .match_indices(':')
        .map(|(index, _)| &path_text[index..])
        .find(|suffix| is_location(suffix))
}

/// `:line[:column]`, then optionally `-` or `–` and `line[:column]`, and
/// nothing more.
fn is_location(suffix: &str) -> bool {
    let Some(rest) = suffix.strip_prefix(':') else {
        return false;
    };
    let (start, end) = match rest.split_once(['-', '–']) {
        Some((start, end)) => (start, Some(end)),
        None => (rest, None),
    };
    is_location_point(start) && end.is_none_or(is_location_point)
}

/// `line` or `line:column`.
fn is_location_point(point: &str) -> bool {
    match point.split_once(':') {
        Some((line, column)) => is_digits(line) && is_digits(column),
        None => is_digits(point),
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// `text` with its `%XX` escapes decoded; unchanged when they do not decode
/// to UTF-8.
fn percent_decode(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).map_or(Cow::Borrowed(text), Cow::Owned)
}

/// Forward slashes throughout, and `\\server\share` as `//server/share`, so
/// prefixes compare the same whatever the separators.
fn normalize_local_link_path_text(path_text: &str) -> String {
    if let Some(rest) = path_text.strip_prefix("\\\\") {
        format!("//{}", rest.replace('\\', "/").trim_start_matches('/'))
    } else {
        path_text.replace('\\', "/")
    }
}

fn is_absolute_local_link_path(path_text: &str) -> bool {
    path_text.starts_with('/')
        || matches!(
            path_text.as_bytes(),
            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        )
}

/// `path_text` without its trailing separators, a root (`/`, `//`, `C:/`)
/// kept whole.
pub(super) fn trim_trailing_local_path_separator(path_text: &str) -> &str {
    if path_text == "/" || path_text == "//" {
        return path_text;
    }
    if matches!(path_text.as_bytes(), [drive, b':', b'/'] if drive.is_ascii_alphabetic()) {
        return path_text;
    }
    path_text.trim_end_matches('/')
}

/// What follows `cwd_text` in `path_text` when the path is strictly under
/// it; `None` for the directory itself, which is shown whole.
fn strip_local_path_prefix<'a>(path_text: &'a str, cwd_text: &str) -> Option<&'a str> {
    let path_text = trim_trailing_local_path_separator(path_text);
    let cwd_text = trim_trailing_local_path_separator(cwd_text);
    if path_text == cwd_text {
        return None;
    }
    if cwd_text == "/" || cwd_text == "//" {
        return path_text.strip_prefix('/');
    }
    path_text
        .strip_prefix(cwd_text)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// A relative path (`~/` included) as it is; an absolute one relative to
/// `cwd` when under it, else whole: the interface's home may not be the
/// home of the host the Agent works on.
fn display_local_link_path(path_text: &str, cwd: Option<&str>) -> String {
    let path_text = normalize_local_link_path_text(path_text);
    if !is_absolute_local_link_path(&path_text) {
        return path_text;
    }
    if let Some(cwd) = cwd {
        let cwd_text = normalize_local_link_path_text(cwd);
        if let Some(stripped) = strip_local_path_prefix(&path_text, &cwd_text) {
            return stripped.to_string();
        }
    }
    path_text
}

#[cfg(test)]
mod tests;
