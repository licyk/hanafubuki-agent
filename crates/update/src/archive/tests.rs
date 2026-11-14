use std::path::PathBuf;

use super::*;
use crate::test_support::FakeArchive;

const ENTRY: &str = "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu/hanafubuki-agent";

#[derive(Clone, Copy, Debug)]
enum Format {
    TarGz,
    Zip,
}

const FORMATS: [Format; 2] = [Format::TarGz, Format::Zip];

struct Staging {
    directory: tempfile::TempDir,
}

impl Staging {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn destination(&self) -> PathBuf {
        self.directory.path().join("staged")
    }

    /// Takes `ENTRY` from `archive` written in `format`, at most `max_bytes`.
    fn take(&self, format: Format, archive: &FakeArchive, max_bytes: u64) -> Result<(), Failure> {
        let path = self.directory.path().join("archive");
        let bytes = match format {
            Format::TarGz => archive.tar_gz(),
            Format::Zip => archive.zip(),
        };
        std::fs::write(&path, bytes).unwrap();
        let destination = self.destination();
        let extraction = Extraction {
            archive: &path,
            archive_name: "the archive",
            entry: ENTRY,
            max_bytes,
            destination: &destination,
            directory: "/the/directory",
        };
        let file = File::open(&path).unwrap();
        match format {
            Format::TarGz => from_tar_gz(file, &extraction),
            Format::Zip => from_zip(file, &extraction),
        }
    }

    /// The staging directory's files: the archive and what was written.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn released(executable: &[u8]) -> FakeArchive {
    FakeArchive::new()
        .directory("hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu/")
        .file(ENTRY, executable)
        .file(
            "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu/LICENSE",
            "GPL-3.0-only",
        )
}

#[test]
fn the_entry_is_taken_by_its_exact_path_from_either_format() {
    for format in FORMATS {
        let staging = Staging::new();
        staging
            .take(format, &released(b"the new executable"), 1 << 20)
            .unwrap();
        assert_eq!(
            std::fs::read(staging.destination()).unwrap(),
            b"the new executable",
            "{format:?}"
        );
    }
}

#[test]
fn other_entries_and_paths_that_leave_the_directory_are_never_written() {
    let archive = FakeArchive::new()
        .file("../escaped", "outside")
        .file("/absolute", "outside")
        .file(
            "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu/../../escaped",
            "outside",
        )
        .symlink(
            "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu/link",
            "/etc/passwd",
        )
        .file("hanafubuki-agent", "a decoy at another path")
        .file(&format!("./{ENTRY}"), "a decoy at another spelling")
        .file(ENTRY, "the new executable")
        .file("after", "never read");
    for format in FORMATS {
        let staging = Staging::new();
        staging.take(format, &archive, 1 << 20).unwrap();
        assert_eq!(
            std::fs::read(staging.destination()).unwrap(),
            b"the new executable"
        );
        assert_eq!(staging.names(), ["archive", "staged"], "{format:?}");
        assert!(!staging
            .directory
            .path()
            .parent()
            .unwrap()
            .join("escaped")
            .exists());
    }
}

#[test]
fn an_archive_without_the_entry_as_a_file_has_none() {
    let cases = [
        FakeArchive::new().file("hanafubuki-agent", "elsewhere"),
        FakeArchive::new().directory(&format!("{ENTRY}/")),
        FakeArchive::new().symlink(ENTRY, "/bin/sh"),
        FakeArchive::new().file(&ENTRY.to_uppercase(), "another case"),
    ];
    for format in FORMATS {
        for archive in &cases {
            let staging = Staging::new();
            let failure = staging.take(format, archive, 1 << 20).unwrap_err();
            assert!(
                matches!(failure, Failure::NoEntry),
                "{format:?} {archive:?}: {failure:?}"
            );
            assert!(!staging.destination().exists());
        }
    }
}

#[test]
fn an_entry_larger_than_the_bound_is_refused() {
    let archive = released(&vec![0; 2 * 1024 * 1024]);
    for format in FORMATS {
        let staging = Staging::new();
        let failure = staging.take(format, &archive, 1024 * 1024).unwrap_err();
        assert!(
            matches!(failure, Failure::NoEntry),
            "{format:?}: {failure:?}"
        );
    }
}

#[test]
fn the_copy_stops_at_the_bound_whatever_the_header_says() {
    let staging = Staging::new();
    let extraction = Extraction {
        archive: Path::new("unused"),
        archive_name: "the archive",
        entry: ENTRY,
        max_bytes: 10,
        destination: &staging.destination(),
        directory: "/the/directory",
    };
    let failure = copy_bounded(&mut &[7u8; 64][..], &extraction).unwrap_err();
    assert!(matches!(failure, Failure::NoEntry), "{failure:?}");
    assert!(std::fs::metadata(staging.destination()).unwrap().len() <= 10);
}

#[test]
fn a_zip_entry_written_with_backslashes_is_found() {
    let staging = Staging::new();
    let archive = FakeArchive::new().file(&ENTRY.replace('/', "\\"), "the new executable");
    staging.take(Format::Zip, &archive, 1 << 20).unwrap();
    assert_eq!(
        std::fs::read(staging.destination()).unwrap(),
        b"the new executable"
    );
}

#[test]
fn a_file_that_is_not_an_archive_does_not_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive");
    std::fs::write(&path, b"<html>a proxy's page</html>").unwrap();
    let destination = directory.path().join("staged");
    let extraction = Extraction {
        archive: &path,
        archive_name: "hanafubuki-agent-9.9.9-x.tar.gz",
        entry: ENTRY,
        max_bytes: 1 << 20,
        destination: &destination,
        directory: "/the/directory",
    };
    let error = extraction.run().unwrap_err();
    assert!(
        matches!(&error, UpdateError::Archive { archive, .. } if archive == "hanafubuki-agent-9.9.9-x.tar.gz"),
        "{error:?}"
    );
    assert!(error.to_string().ends_with("nothing was changed."));
    for format in FORMATS {
        let failure = match format {
            Format::TarGz => from_tar_gz(File::open(&path).unwrap(), &extraction),
            Format::Zip => from_zip(File::open(&path).unwrap(), &extraction),
        }
        .unwrap_err();
        assert!(
            matches!(failure, Failure::Read(_)),
            "{format:?}: {failure:?}"
        );
    }
}

#[test]
fn a_missing_entry_names_the_archive_and_the_entry() {
    let error = Extraction {
        archive: Path::new("unused"),
        archive_name: "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu.tar.gz",
        entry: ENTRY,
        max_bytes: 0,
        destination: Path::new("unused"),
        directory: "/the/directory",
    }
    .error(Failure::NoEntry);
    assert_eq!(
        error.to_string(),
        format!(
            "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu.tar.gz does not contain {ENTRY} as \
             a file; nothing was changed."
        )
    );
}
