//! The executable taken from the release archive (D4): one entry, by its
//! exact path, a regular file no larger than a bound, written to a path the
//! updater chooses. Nothing else in the archive is ever written, so no entry
//! can reach outside the staging directory, whatever its name.
//!
//! `.tar.gz` on Unix and `.zip` on Windows, as `release.yml` packages them;
//! the tests read both everywhere.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

use crate::error::UpdateError;

/// The entry to take from an archive, and where it goes.
pub(crate) struct Extraction<'a> {
    pub(crate) archive: &'a Path,
    /// The archive's asset name, for a message.
    pub(crate) archive_name: &'a str,
    /// The entry's path inside the archive ([`crate::entry_path`]).
    pub(crate) entry: &'a str,
    /// The largest entry taken, uncompressed.
    pub(crate) max_bytes: u64,
    pub(crate) destination: &'a Path,
    /// The executable's directory, for a message.
    pub(crate) directory: &'a str,
}

/// Why an entry was not taken.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The archive does not read.
    Read(String),
    /// No regular file at the entry's path, or one over the bound.
    NoEntry,
    /// The entry could not be written.
    Write(io::Error),
}

impl Extraction<'_> {
    /// Writes the entry to the destination and syncs it.
    pub(crate) fn run(&self) -> Result<(), UpdateError> {
        let archive = File::open(self.archive)
            .map_err(|error| self.error(Failure::Read(error.to_string())))?;
        #[cfg(unix)]
        let taken = from_tar_gz(archive, self);
        #[cfg(windows)]
        let taken = from_zip(archive, self);
        taken.map_err(|failure| self.error(failure))
    }

    fn error(&self, failure: Failure) -> UpdateError {
        match failure {
            Failure::Read(reason) => UpdateError::Archive {
                archive: self.archive_name.to_string(),
                reason,
            },
            Failure::NoEntry => UpdateError::NoEntry {
                archive: self.archive_name.to_string(),
                entry: self.entry.to_string(),
            },
            Failure::Write(error) => UpdateError::Write {
                what: "the new executable",
                directory: self.directory.to_string(),
                reason: error.to_string(),
            },
        }
    }
}

/// The entry of a gzipped tar archive, matched on its full path (GNU and
/// PAX long names included).
#[cfg(any(unix, test))]
pub(crate) fn from_tar_gz(archive: File, extraction: &Extraction<'_>) -> Result<(), Failure> {
    let read = |error: io::Error| Failure::Read(error.to_string());
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(archive)));
    for entry in tar.entries().map_err(read)? {
        let mut entry = entry.map_err(read)?;
        if entry.path_bytes().as_ref() != extraction.entry.as_bytes() {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > extraction.max_bytes {
            return Err(Failure::NoEntry);
        }
        return copy_bounded(&mut entry, extraction);
    }
    Err(Failure::NoEntry)
}

/// The entry of a zip archive, matched on its full name (a `\` read as
/// `/`, as an archiver on Windows might write it).
#[cfg(any(windows, test))]
pub(crate) fn from_zip(archive: File, extraction: &Extraction<'_>) -> Result<(), Failure> {
    let read = |error: zip::result::ZipError| Failure::Read(error.to_string());
    let mut zip = zip::ZipArchive::new(BufReader::new(archive)).map_err(read)?;
    let wanted = extraction.entry.as_bytes();
    let index = zip
        .file_names_raw()
        .position(|name| {
            name.len() == wanted.len()
                && name
                    .iter()
                    .map(|&byte| if byte == b'\\' { b'/' } else { byte })
                    .eq(wanted.iter().copied())
        })
        .ok_or(Failure::NoEntry)?;
    let mut file = zip.by_index(index).map_err(read)?;
    if !file.is_file() || file.size() > extraction.max_bytes {
        return Err(Failure::NoEntry);
    }
    copy_bounded(&mut file, extraction)
}

/// Copies `entry` to the destination, failing past the bound whatever the
/// archive's header said, and syncs it.
fn copy_bounded(entry: &mut dyn Read, extraction: &Extraction<'_>) -> Result<(), Failure> {
    let mut file = File::create(extraction.destination).map_err(Failure::Write)?;
    let mut buffer = vec![0; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let count = match entry.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Failure::Read(error.to_string())),
        };
        written += count as u64;
        if written > extraction.max_bytes {
            return Err(Failure::NoEntry);
        }
        file.write_all(&buffer[..count]).map_err(Failure::Write)?;
    }
    file.sync_all().map_err(Failure::Write)
}

#[cfg(test)]
mod tests;
