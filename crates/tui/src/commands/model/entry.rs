//! The pair typed in: the Provider's id and the model's, for a model the
//! Provider does not list (or one whose Provider is not listed at all).

use crate::bottom_pane::{FormItem, FormView};

const PROVIDER: &str = "provider";
const MODEL: &str = "model";
pub(super) const TAKE: &str = "take";

/// The row of the list that opens it.
pub(super) const LABEL: &str = "Enter a provider and model…";

pub(super) fn form() -> FormView {
    FormView::new(
        "Enter a provider and model",
        vec![
            FormItem::text(PROVIDER, "Provider", "").placeholder("provider-1"),
            FormItem::text(MODEL, "Model", "").placeholder("gpt-5.1"),
            FormItem::action(TAKE, "Add this model"),
        ],
    )
}

/// The pair `form` holds as `PROVIDER/MODEL`, or why it is not one yet.
pub(super) fn pair(form: &mut FormView) -> Option<String> {
    let provider = form.text(PROVIDER).trim().to_string();
    let model = form.text(MODEL).trim().to_string();
    if provider.is_empty() {
        form.set_error(PROVIDER, "Enter the Provider's id.");
        return None;
    }
    if model.is_empty() {
        form.set_error(MODEL, "Enter the model's id.");
        return None;
    }
    Some(format!("{provider}/{model}"))
}

/// `pair` split at its first `/`, as the collection reads it.
pub(super) fn split(pair: &str) -> (&str, &str) {
    pair.split_once('/').unwrap_or((pair, ""))
}
