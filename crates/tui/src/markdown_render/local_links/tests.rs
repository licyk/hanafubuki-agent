// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn trailing_separator_trimming_preserves_local_roots() {
    let paths = ["/", "//", "C:/", "dir/", "//server/share/", "C:/dir/"];
    assert_eq!(
        paths.map(trim_trailing_local_path_separator),
        ["/", "//", "C:/", "dir", "//server/share", "C:/dir"]
    );
}

/// The two location shapes Codex matches with regular expressions.
#[test]
fn location_suffixes_match_codex_patterns() {
    for (path, suffix) in [
        ("src/lib.rs:12", Some(":12")),
        ("src/lib.rs:12:3", Some(":12:3")),
        ("src/lib.rs:12-14", Some(":12-14")),
        ("src/lib.rs:12:3-14:9", Some(":12:3-14:9")),
        ("src/lib.rs:12–14", Some(":12–14")),
        ("C:/repo/lib.rs:7", Some(":7")),
        ("C:/repo/lib.rs", None),
        ("src/lib.rs:", None),
        ("src/lib.rs:12:", None),
        ("src/lib.rs:1:2:3", Some(":2:3")),
        ("src/lib.rs:x1", None),
    ] {
        assert_eq!(colon_location_suffix(path), suffix, "{path}");
    }
    for (fragment, suffix) in [
        ("L12", Some(":12")),
        ("L12C3", Some(":12:3")),
        ("L12C3-L14C9", Some(":12:3-14:9")),
        ("L12-L14", Some(":12-14")),
        ("section", None),
        ("L", None),
        ("L12C", None),
        ("12", None),
    ] {
        assert_eq!(
            hash_location_suffix(fragment).as_deref(),
            suffix,
            "{fragment}"
        );
    }
}

#[test]
fn percent_decoding_keeps_text_that_is_not_utf8() {
    assert_eq!(percent_decode("My%20File.rs"), "My File.rs");
    assert_eq!(percent_decode("R%C3%A9sum%C3%A9"), "Résumé");
    assert_eq!(percent_decode("bad%FF"), "bad%FF");
    assert_eq!(percent_decode("100%"), "100%");
    assert_eq!(percent_decode("%2520"), "%20");
}

#[test]
fn file_urls_keep_their_own_convention() {
    let path = |url: &str| file_url_path(&Url::parse(url).unwrap());
    assert_eq!(path("file:///repo/src/lib.rs"), "/repo/src/lib.rs");
    assert_eq!(path("file:///C:/Repo/My%20File.rs"), "C:/Repo/My File.rs");
    assert_eq!(path("file://server/share/a.rs"), "//server/share/a.rs");
}
