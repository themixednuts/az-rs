//! Immutable lookup for native literal-keyed script templates.

use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

use crate::{SlayerProgram, SlayerScriptLiteral};

/// Validated generic template registry keyed by the native literal CRC.
///
/// The registry owns no asset resolver: project cooking or static module code
/// supplies already-validated programs, and instance construction still starts
/// every layer empty.
#[derive(Debug, Clone)]
pub struct ScriptTemplateRegistry<O, E = ()> {
    templates: BTreeMap<SlayerScriptLiteral, Arc<SlayerProgram<O, E>>>,
}

impl<O, E> ScriptTemplateRegistry<O, E> {
    /// Builds an immutable registry and rejects ambiguous literal collisions.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptTemplateRegistryError::DuplicateLiteral`] when two
    /// entries carry the same literal CRC, because the second would silently
    /// replace the first.
    pub fn new(
        templates: impl IntoIterator<Item = (SlayerScriptLiteral, Arc<SlayerProgram<O, E>>)>,
    ) -> Result<Self, ScriptTemplateRegistryError> {
        let mut by_literal = BTreeMap::new();
        for (literal, program) in templates {
            if by_literal.insert(literal, program).is_some() {
                return Err(ScriptTemplateRegistryError::DuplicateLiteral { literal });
            }
        }
        Ok(Self {
            templates: by_literal,
        })
    }

    /// Resolves one validated program without interpreting a path or source bag.
    #[must_use]
    pub fn program(&self, literal: SlayerScriptLiteral) -> Option<&Arc<SlayerProgram<O, E>>> {
        self.templates.get(&literal)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}

/// Why a literal-keyed template registry is ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScriptTemplateRegistryError {
    #[error("duplicate SlayerScript template literal {literal:?}")]
    DuplicateLiteral { literal: SlayerScriptLiteral },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_exact_literals_and_rejects_collisions() {
        let program = Arc::new(
            SlayerProgram::<(), ()>::new(Vec::new(), Vec::new(), crate::StateTable::empty())
                .unwrap(),
        );
        let literal = SlayerScriptLiteral::from("AchievementEvents");
        let registry = ScriptTemplateRegistry::new([(literal, Arc::clone(&program))]).unwrap();

        assert!(Arc::ptr_eq(registry.program(literal).unwrap(), &program));
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert_eq!(
            ScriptTemplateRegistry::new([
                (literal, Arc::clone(&program)),
                (literal, Arc::clone(&program)),
            ])
            .unwrap_err(),
            ScriptTemplateRegistryError::DuplicateLiteral { literal }
        );
    }
}
