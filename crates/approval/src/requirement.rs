/// Stable permission classification produced before a reviewer is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRequirement<T> {
    NotRequired,
    ReviewRequired(T),
    Forbidden { reason: String },
}

impl<T> PermissionRequirement<T> {
    pub const fn is_not_required(&self) -> bool {
        matches!(self, Self::NotRequired)
    }

    pub const fn is_review_required(&self) -> bool {
        matches!(self, Self::ReviewRequired(_))
    }
}
