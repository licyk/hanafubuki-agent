use super::{history::Evidence, AdvisorOptions};
use hanafubuki_agent_foundation::paths::PathKey;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct Previous {
    pub id: String,
    pub target: (String, String),
    pub answer: String,
    pub prompt: String,
    pub evidence: Arc<Evidence>,
    pub created: Instant,
    pub root: PathKey,
    pub trusted: bool,
    pub options: Arc<AdvisorOptions>,
    pub user_item: Option<u64>,
    pub claimed: bool,
}

impl Previous {
    pub(super) fn fresh(&self) -> bool {
        self.created.elapsed() < Duration::from_secs(300)
    }
    pub(super) fn mentions(&self, path: &str) -> bool {
        self.answer.match_indices(path).any(|(i, _)| {
            let boundary = |c: char| c.is_whitespace() || "\"'`([]),;:!?".contains(c);
            self.answer[..i].chars().next_back().is_none_or(boundary)
                && self.answer[i + path.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| {
                        boundary(c)
                            || c == '.'
                                && self.answer[i + path.len() + 1..]
                                    .chars()
                                    .next()
                                    .is_none_or(char::is_whitespace)
                    })
        })
    }
}

pub(super) fn next_id() -> String {
    format!(
        "advice-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
