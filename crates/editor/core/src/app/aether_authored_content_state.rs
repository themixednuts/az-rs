//! Aether-local presentation state layered over the published authored outline.

use std::collections::{BTreeMap, BTreeSet};

use az_editor_ui::panels::EditorAuthoredOutline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoredOutlineExpansion {
    Expanded,
    Collapsed,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::app) struct AetherAuthoredExpansion<'a> {
    overrides: &'a BTreeMap<String, AuthoredOutlineExpansion>,
}

impl AetherAuthoredExpansion<'_> {
    pub(in crate::app) fn is_open(self, key: &str, default_open: bool) -> bool {
        if key.is_empty() {
            return default_open;
        }
        match self.overrides.get(key) {
            Some(AuthoredOutlineExpansion::Expanded) => true,
            Some(AuthoredOutlineExpansion::Collapsed) => false,
            None => default_open,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::app) struct AetherAuthoredSourcePaths<'a> {
    redirects: &'a BTreeMap<String, String>,
}

impl<'content> AetherAuthoredSourcePaths<'content> {
    pub(in crate::app) fn remapped(&self, source_path: &'content str) -> &'content str {
        let mut current = source_path;
        let mut seen = BTreeSet::new();
        while seen.insert(current.to_owned()) {
            let Some(next) = self.redirects.get(current) else {
                break;
            };
            current = next;
        }
        current
    }
}

/// This type has no selection, document, or client state: the outline is
/// published by the existing controller and passed only to stale-redirect
/// cleanup.
#[derive(Debug, Clone, Default)]
pub(super) struct AetherAuthoredContentState {
    expansion: BTreeMap<String, AuthoredOutlineExpansion>,
    source_path_redirects: BTreeMap<String, String>,
}

impl AetherAuthoredContentState {
    pub(super) fn expansion(&self) -> AetherAuthoredExpansion<'_> {
        AetherAuthoredExpansion {
            overrides: &self.expansion,
        }
    }

    pub(super) fn source_paths(&self) -> AetherAuthoredSourcePaths<'_> {
        AetherAuthoredSourcePaths {
            redirects: &self.source_path_redirects,
        }
    }

    pub(super) fn set_expanded(&mut self, key: &str, open: bool) -> bool {
        if key.is_empty() {
            return false;
        }
        let next = if open {
            AuthoredOutlineExpansion::Expanded
        } else {
            AuthoredOutlineExpansion::Collapsed
        };
        if self.expansion.get(key) == Some(&next) {
            return false;
        }
        self.expansion.insert(key.to_owned(), next);
        true
    }

    pub(super) fn record_source_path_move(&mut self, from: String, to: String) {
        if from == to {
            self.source_path_redirects.remove(&from);
            return;
        }
        for value in self.source_path_redirects.values_mut() {
            if *value == from {
                *value = to.clone();
            }
        }
        self.source_path_redirects.insert(from, to);
        loop {
            let updates = self
                .source_path_redirects
                .iter()
                .filter_map(|(from, to)| {
                    self.source_path_redirects
                        .get(to)
                        .filter(|next| *next != to)
                        .map(|next| (from.clone(), next.clone()))
                })
                .collect::<Vec<_>>();
            if updates.is_empty() {
                break;
            }
            for (from, to) in updates {
                self.source_path_redirects.insert(from, to);
            }
        }
        self.source_path_redirects.retain(|from, to| from != to);
    }

    pub(super) fn clear_resolved_source_path_moves(
        &mut self,
        outline: &EditorAuthoredOutline,
    ) -> bool {
        let paths = outline
            .data
            .documents
            .iter()
            .flat_map(|document| [&document.document_id, &document.source_path])
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect::<BTreeSet<_>>();
        let before = self.source_path_redirects.len();
        self.source_path_redirects
            .retain(|from, to| paths.contains(from) && !paths.contains(to));
        before != self.source_path_redirects.len()
    }
}
