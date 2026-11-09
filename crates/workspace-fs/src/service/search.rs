use std::io;

use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkFinish, SinkMatch};
use ignore::WalkBuilder;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchStop {
    Cancelled,
    Deadline,
    ResultLimit,
}

struct FileSearchSink<'a> {
    matcher: &'a RegexMatcher,
    path: &'a str,
    cancellation: &'a CancellationToken,
    deadline: Instant,
    max_results: usize,
    matches: Vec<SearchMatch>,
    stop: Option<SearchStop>,
    binary: bool,
}

impl FileSearchSink<'_> {
    fn should_continue(&mut self) -> bool {
        if self.cancellation.is_cancelled() {
            self.stop = Some(SearchStop::Cancelled);
            false
        } else if Instant::now() >= self.deadline {
            self.stop = Some(SearchStop::Deadline);
            false
        } else {
            true
        }
    }
}

impl Sink for FileSearchSink<'_> {
    type Error = io::Error;

    fn begin(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> {
        Ok(self.should_continue())
    }

    fn matched(
        &mut self,
        _searcher: &Searcher,
        matched_line: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        if !self.should_continue() {
            return Ok(false);
        }

        let line = usize::try_from(matched_line.line_number().unwrap_or(1))
            .map_err(|_| io::Error::other("search line number exceeds platform limits"))?;
        let bytes = matched_line.bytes();
        let preview_text = String::from_utf8_lossy(bytes);
        let preview = bounded_preview(preview_text.trim_end_matches(['\r', '\n']));
        let remaining = self.max_results.saturating_sub(self.matches.len());
        if remaining == 0 {
            self.stop = Some(SearchStop::ResultLimit);
            return Ok(true);
        }
        let mut columns = Vec::new();
        self.matcher
            .find_iter(bytes, |matched| {
                columns.push(matched.start() + 1);
                columns.len() < remaining
            })
            .map_err(|error| io::Error::other(error.to_string()))?;

        self.matches
            .extend(columns.into_iter().map(|column| SearchMatch {
                path: self.path.to_string(),
                line,
                column,
                preview: preview.clone(),
            }));
        if self.matches.len() >= self.max_results {
            self.stop = Some(SearchStop::ResultLimit);
        }
        Ok(true)
    }

    fn binary_data(
        &mut self,
        _searcher: &Searcher,
        _binary_byte_offset: u64,
    ) -> Result<bool, Self::Error> {
        self.binary = true;
        self.matches.clear();
        if self.stop == Some(SearchStop::ResultLimit) {
            self.stop = None;
        }
        Ok(false)
    }

    fn finish(&mut self, _searcher: &Searcher, finish: &SinkFinish) -> Result<(), Self::Error> {
        if finish.binary_byte_offset().is_some() {
            self.binary = true;
            self.matches.clear();
            if self.stop == Some(SearchStop::ResultLimit) {
                self.stop = None;
            }
        }
        Ok(())
    }
}

impl WorkspaceFsService {
    pub(super) fn search_text(
        &self,
        root: ExistingPathPlan,
        matcher: &RegexMatcher,
        max_depth: usize,
        max_results: usize,
        max_bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        revalidate_existing(&root)?;
        let deadline = Instant::now() + SEARCH_DEADLINE;
        let mut matches = Vec::new();
        let mut scanned_files = 0usize;
        let mut scanned_bytes = 0usize;
        let mut truncated = false;
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\0'))
            .build();
        let mut walker = WalkBuilder::new(root.resolved.system());
        walker
            .standard_filters(true)
            .require_git(false)
            .add_custom_ignore_filename(".rgignore")
            .follow_links(false)
            .max_depth(Some(max_depth))
            .sort_by_file_name(|left, right| left.cmp(right));

        for entry in walker.build() {
            check_cancel(cancellation)?;
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            let entry = entry.map_err(|_| {
                WorkspaceFsError::Io(format!(
                    "walk search tree under {}",
                    root.resolved.native_text()
                ))
            })?;
            if entry.depth() == 0 {
                continue;
            }
            if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
                continue;
            }

            let child = ResolvedPath::resolve(entry.path())
                .map_err(|error| WorkspaceFsError::Resolution(error.to_string()))?;
            if !child.starts_with(&root.resolved) {
                return Err(WorkspaceFsError::PathChanged);
            }
            self.policy().ensure_unprotected(&child)?;
            let metadata = fs::symlink_metadata(child.system()).map_err(|error| {
                WorkspaceFsError::Io(format!("inspect {}: {error}", child.native_text()))
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if entry.depth() == max_depth {
                    truncated = true;
                }
                continue;
            }
            if !metadata.is_file() {
                truncated = true;
                continue;
            }
            if metadata.len() > MAX_SEARCH_FILE_BYTES {
                truncated = true;
                continue;
            }
            let remaining_bytes = max_bytes.saturating_sub(scanned_bytes);
            if metadata.len() as usize > remaining_bytes {
                truncated = true;
                break;
            }

            let identity = identity_from_metadata(&child, &metadata)?;
            let file_plan = ExistingPathPlan {
                resolved: child,
                identity,
            };
            revalidate_existing(&file_plan)?;
            let path = file_plan.resolved.native_text().to_string();
            let mut sink = FileSearchSink {
                matcher,
                path: &path,
                cancellation,
                deadline,
                max_results: max_results - matches.len(),
                matches: Vec::new(),
                stop: None,
                binary: false,
            };
            searcher
                .search_path(matcher, file_plan.resolved.system(), &mut sink)
                .map_err(|error| WorkspaceFsError::Io(format!("search {}: {error}", path)))?;
            revalidate_existing(&file_plan)?;
            scanned_bytes = scanned_bytes.saturating_add(metadata.len() as usize);
            scanned_files = scanned_files.saturating_add(1);

            if !sink.binary {
                matches.append(&mut sink.matches);
            }
            match sink.stop {
                Some(SearchStop::Cancelled) => return Err(WorkspaceFsError::Cancelled),
                Some(SearchStop::Deadline | SearchStop::ResultLimit) => {
                    truncated = true;
                    break;
                }
                None => {}
            }
        }

        revalidate_existing(&root)?;
        Ok(WorkspaceFsResult::SearchResults(SearchResults {
            root: root.identity.path,
            matches,
            scanned_files,
            scanned_bytes,
            truncated,
        }))
    }
}
