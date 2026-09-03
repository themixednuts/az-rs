//! Compiled Mannequin controller definitions.

use az_core::crc::Crc32;
use serde::{Deserialize, Serialize};

use super::{
    FragmentDefinition, FragmentFlags, FragmentId, FragmentTagState, MannequinDatabaseDefinition,
    ScopeContextId, ScopeId, ScopeMask, SubContextId, TagDefinition, TagState,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentScopeMask {
    pub tags: FragmentTagState,
    pub scopes: ScopeMask,
}

/// Cry's `TTagSortedList<ActionScopes>` used by one fragment definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentScopeMaskList {
    entries: Vec<FragmentScopeMask>,
}

impl Default for FragmentScopeMaskList {
    fn default() -> Self {
        Self {
            entries: vec![FragmentScopeMask {
                tags: FragmentTagState::default(),
                scopes: ScopeMask::ALL,
            }],
        }
    }
}

impl FragmentScopeMaskList {
    #[must_use]
    pub fn entries(&self) -> &[FragmentScopeMask] {
        &self.entries
    }

    pub fn insert(&mut self, tags: FragmentTagState, scopes: ScopeMask) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.tags == tags) {
            entry.scopes = scopes;
        } else {
            self.entries.push(FragmentScopeMask { tags, scopes });
        }
    }

    pub fn sort(&mut self, global: &TagDefinition, fragment: Option<&TagDefinition>) {
        let combined = fragment.map(|fragment| global.combined_priority_tallies(fragment));
        self.entries.sort_by_key(|entry| {
            let global_score = combined.as_deref().map_or_else(
                || global.rate(entry.tags.global_tags),
                |tallies| global.rate_with_tallies(entry.tags.global_tags, tallies),
            );
            let fragment_score = fragment.map_or(0, |definition| {
                combined.as_deref().map_or_else(
                    || definition.rate(entry.tags.fragment_tags),
                    |tallies| definition.rate_with_tallies(entry.tags.fragment_tags, tallies),
                )
            });
            std::cmp::Reverse(global_score.saturating_add(fragment_score))
        });
    }

    #[must_use]
    pub fn best_match(
        &self,
        query: FragmentTagState,
        global: &TagDefinition,
        fragment: Option<&TagDefinition>,
    ) -> Option<ScopeMask> {
        self.entries
            .iter()
            .find(|entry| {
                global.contains(query.global_tags, entry.tags.global_tags)
                    && fragment.is_none_or(|definition| {
                        definition.contains(query.fragment_tags, entry.tags.fragment_tags)
                    })
            })
            .map(|entry| entry.scopes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerFragmentDefinition {
    pub scope_masks: FragmentScopeMaskList,
    pub flags: FragmentFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeDefinition {
    pub context: ScopeContextId,
    pub layer: u32,
    pub layer_count: u32,
    pub additional_tags: TagState,
    /// Lowercase CRC of Cry's authored `scopeAlias` (or the scope name).
    pub alias: Crc32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeContextDefinition {
    pub shared_tags: TagState,
    pub additional_tags: TagState,
}

impl Default for ScopeContextDefinition {
    fn default() -> Self {
        Self {
            shared_tags: TagState::FULL,
            additional_tags: TagState::EMPTY,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubContextDefinition {
    pub scopes: ScopeMask,
    pub additional_tags: TagState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerDefinition {
    pub global_tags: TagDefinition,
    pub fragments: FragmentDefinition,
    pub fragment_definitions: Vec<ControllerFragmentDefinition>,
    pub scopes: Vec<ScopeDefinition>,
    pub scope_contexts: Vec<ScopeContextDefinition>,
    pub sub_contexts: Vec<SubContextDefinition>,
}

impl ControllerDefinition {
    /// Sorts every fragment's scope masks into Mannequin's tag-priority order.
    ///
    /// # Panics
    ///
    /// Panics if a fragment's position in `fragment_definitions` is not a valid
    /// [`FragmentId`], which can only happen for a definition that failed
    /// validation.
    pub fn sort_fragment_scope_masks(&mut self) {
        for (index, fragment) in self.fragment_definitions.iter_mut().enumerate() {
            let id = FragmentId::new(index).expect("validated controller fragment index");
            fragment
                .scope_masks
                .sort(&self.global_tags, self.fragments.tag_definition(id));
        }
    }

    #[must_use]
    pub fn fragment(&self, fragment: FragmentId) -> Option<&ControllerFragmentDefinition> {
        self.fragment_definitions.get(fragment.index())
    }

    #[must_use]
    pub fn scope(&self, scope: ScopeId) -> Option<&ScopeDefinition> {
        self.scopes.get(scope.index())
    }

    #[must_use]
    pub fn scope_context(&self, context: ScopeContextId) -> Option<&ScopeContextDefinition> {
        self.scope_contexts.get(context.index())
    }

    #[must_use]
    pub fn sub_context(&self, context: SubContextId) -> Option<&SubContextDefinition> {
        self.sub_contexts.get(context.index())
    }

    #[must_use]
    pub fn scope_mask(
        &self,
        fragment: FragmentId,
        tags: FragmentTagState,
        sub_context: Option<SubContextId>,
    ) -> ScopeMask {
        let mut scopes = self
            .fragment(fragment)
            .and_then(|definition| {
                definition.scope_masks.best_match(
                    tags,
                    &self.global_tags,
                    self.fragments.tag_definition(fragment),
                )
            })
            .unwrap_or_default();
        if let Some(sub_context) = sub_context.and_then(|id| self.sub_context(id)) {
            scopes |= sub_context.scopes;
        }
        scopes
    }

    /// Yields one [`super::ActionScope`] per declared scope, in scope order.
    ///
    /// # Panics
    ///
    /// The returned iterator panics if a scope's position in `scopes` is not a
    /// valid [`ScopeId`], which can only happen for a definition that failed
    /// validation.
    pub fn action_scopes(&self) -> impl Iterator<Item = super::ActionScope> + '_ {
        self.scopes.iter().enumerate().map(|(index, scope)| {
            super::ActionScope::new(
                ScopeId::new(index).expect("validated scope index"),
                scope.context,
                scope.layer,
                scope.layer_count,
            )
        })
    }
}

impl MannequinDatabaseDefinition for ControllerDefinition {
    fn global_tag_definition(&self) -> &TagDefinition {
        &self.global_tags
    }

    fn fragment_tag_definition(&self, fragment: FragmentId) -> Option<&TagDefinition> {
        self.fragments.tag_definition(fragment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_scope_override_wins_and_sub_context_is_union() {
        let mut global = TagDefinition::builder();
        let combat = global.add_tag(None, 2).unwrap();
        let global = global.build().unwrap();
        let fragments = FragmentDefinition::new(vec![None]);
        let fragment = FragmentId::new(0).unwrap();
        let mut definition = ControllerDefinition {
            global_tags: global,
            fragments,
            fragment_definitions: vec![ControllerFragmentDefinition::default()],
            sub_contexts: vec![SubContextDefinition {
                scopes: ScopeMask::from_bits_retain(0b100),
                additional_tags: TagState::EMPTY,
            }],
            ..ControllerDefinition::default()
        };
        definition.fragment_definitions[0]
            .scope_masks
            .insert(FragmentTagState::default(), ScopeMask::from_bits_retain(1));
        let mut tags = FragmentTagState::default();
        definition
            .global_tags
            .set(&mut tags.global_tags, combat, true)
            .unwrap();
        definition.fragment_definitions[0]
            .scope_masks
            .insert(tags, ScopeMask::from_bits_retain(0b10));
        definition.sort_fragment_scope_masks();

        assert_eq!(
            definition
                .scope_mask(fragment, tags, SubContextId::new(0))
                .bits(),
            0b110
        );
    }
}
