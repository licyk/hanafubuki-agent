//! A set of ids that iterates in insertion order, as JavaScript's `Set`
//! does: the kernel's block coverage and its warnings list ids in the order
//! its sets received them.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct OrderedIds {
    slots: Vec<Option<String>>,
    positions: HashMap<String, usize>,
}

impl OrderedIds {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Inserts `id` at the end unless it is present; a removed id inserted
    /// again goes to the end.
    pub(crate) fn insert(&mut self, id: &str) -> bool {
        if self.positions.contains_key(id) {
            return false;
        }
        self.positions.insert(id.to_owned(), self.slots.len());
        self.slots.push(Some(id.to_owned()));
        true
    }

    pub(crate) fn remove(&mut self, id: &str) -> bool {
        match self.positions.remove(id) {
            Some(position) => {
                if let Some(slot) = self.slots.get_mut(position) {
                    *slot = None;
                }
                true
            }
            None => false,
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().filter_map(|slot| slot.as_deref())
    }

    pub(crate) fn to_vec(&self) -> Vec<String> {
        self.iter().map(str::to_owned).collect()
    }
}

impl<'a> FromIterator<&'a str> for OrderedIds {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut ids = Self::new();
        for id in iter {
            ids.insert(id);
        }
        ids
    }
}
