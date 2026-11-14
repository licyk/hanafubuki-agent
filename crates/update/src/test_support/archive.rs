//! Release archives for the fake release: laid out as `release.yml`
//! packages them, or hostile, in this platform's format (`.tar.gz` on
//! Unix, `.zip` on Windows; the crate's own tests write both).

use semver::Version;

use crate::target::{asset_name, entry_path, this_target};

/// One entry of an archive, its path written exactly as given.
#[derive(Clone, Debug)]
enum FakeEntry {
    File { path: String, body: Vec<u8> },
    Directory { path: String },
    Symlink { path: String, target: String },
}

/// An archive's entries, in order.
#[derive(Clone, Debug, Default)]
pub struct FakeArchive {
    entries: Vec<FakeEntry>,
}

impl FakeArchive {
    pub fn new() -> Self {
        Self::default()
    }

    /// `version`'s archive for this target as `release.yml` makes it: the
    /// directory, then `executable` at [`entry_path`] and the licence files
    /// beside it.
    pub fn release(version: &Version, executable: impl Into<Vec<u8>>) -> Self {
        let entry = entry_path(version, release_target());
        let directory = entry.rsplit_once('/').map(|(directory, _)| directory);
        let directory = directory.unwrap_or_default().to_string();
        Self::new()
            .directory(&format!("{directory}/"))
            .file(&entry, executable)
            .file(&format!("{directory}/LICENSE"), "GPL-3.0-only")
            .file(&format!("{directory}/NOTICE"), "hanafubuki-agent")
            .file(
                &format!("{directory}/THIRD-PARTY-LICENSES.txt"),
                "Third-party licenses",
            )
    }

    /// The name of `version`'s archive for this target.
    pub fn name(version: &Version) -> String {
        asset_name(version, release_target())
    }

    pub fn file(mut self, path: &str, body: impl Into<Vec<u8>>) -> Self {
        self.entries.push(FakeEntry::File {
            path: path.to_string(),
            body: body.into(),
        });
        self
    }

    pub fn directory(mut self, path: &str) -> Self {
        self.entries.push(FakeEntry::Directory {
            path: path.to_string(),
        });
        self
    }

    pub fn symlink(mut self, path: &str, target: &str) -> Self {
        self.entries.push(FakeEntry::Symlink {
            path: path.to_string(),
            target: target.to_string(),
        });
        self
    }

    /// The archive in this platform's format.
    pub fn bytes(&self) -> Vec<u8> {
        #[cfg(unix)]
        let bytes = self.tar_gz();
        #[cfg(windows)]
        let bytes = self.zip();
        bytes
    }

    /// A gzipped tar archive, each entry's path written as it is (`..` and
    /// absolute paths included), mode 755.
    #[cfg(any(unix, test))]
    pub fn tar_gz(&self) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for entry in &self.entries {
            let mut header = tar::Header::new_gnu();
            let (path, body): (&str, &[u8]) = match entry {
                FakeEntry::File { path, body } => {
                    header.set_entry_type(tar::EntryType::Regular);
                    (path, body)
                }
                FakeEntry::Directory { path } => {
                    header.set_entry_type(tar::EntryType::Directory);
                    (path, &[])
                }
                FakeEntry::Symlink { path, target } => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    raw_field(&mut header.as_old_mut().linkname, target);
                    (path, &[])
                }
            };
            raw_field(&mut header.as_old_mut().name, path);
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, body).expect("a tar entry");
        }
        builder
            .into_inner()
            .and_then(|encoder| encoder.finish())
            .expect("a tar.gz archive")
    }

    /// A zip archive, Deflate-compressed, each entry's name written as it is.
    #[cfg(any(windows, test))]
    pub fn zip(&self) -> Vec<u8> {
        use std::io::Write;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for entry in &self.entries {
            match entry {
                FakeEntry::File { path, body } => {
                    writer.start_file(path, options).expect("a zip entry");
                    writer.write_all(body).expect("a zip entry's body");
                }
                FakeEntry::Directory { path } => writer
                    .add_directory(path, options)
                    .expect("a zip directory"),
                FakeEntry::Symlink { path, target } => writer
                    .add_symlink(path, target, options)
                    .expect("a zip symlink"),
            }
        }
        writer.finish().expect("a zip archive").into_inner()
    }
}

/// The target releases are built for here; a test needs one.
fn release_target() -> &'static str {
    this_target().expect("the tests run on a release target")
}

/// `value` into a fixed-size header field, as is.
#[cfg(any(unix, test))]
fn raw_field(field: &mut [u8], value: &str) {
    assert!(value.len() < field.len(), "{value} fits a tar header field");
    field.fill(0);
    field[..value.len()].copy_from_slice(value.as_bytes());
}
