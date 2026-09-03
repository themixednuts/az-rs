//! Pure release-pin validation used by the STDB module and host-side tests.

/// Rejects a release pin that names no release or no projection.
///
/// # Errors
///
/// Returns the message `game_data_release_id must be non-empty` or
/// `game_data_projection_hash must be non-empty` when the corresponding
/// argument is empty or all whitespace.
pub fn validate_release_pins_non_empty(
    game_data_release_id: &str,
    game_data_projection_hash: &str,
) -> Result<(), String> {
    if game_data_release_id.trim().is_empty() {
        return Err("game_data_release_id must be non-empty".into());
    }
    if game_data_projection_hash.trim().is_empty() {
        return Err("game_data_projection_hash must be non-empty".into());
    }
    Ok(())
}

/// Checks that a character's release pin matches the world-server node it joins.
///
/// # Errors
///
/// Returns a message naming `node_id` when the node itself pins no release ID
/// or no projection hash, and a message naming both sides when the character's
/// release ID or projection hash differs from the node's. All five arguments
/// are compared after trimming.
pub fn validate_release_against_world_node(
    node_release_id: &str,
    node_projection_hash: &str,
    character_release_id: &str,
    character_projection_hash: &str,
    node_id: &str,
) -> Result<(), String> {
    let node_release_id = node_release_id.trim();
    if node_release_id.is_empty() {
        return Err(format!(
            "world-server node `{node_id}` has no game_data_release_id; heartbeat must pin an active GameData release"
        ));
    }
    let node_hash = node_projection_hash.trim();
    if node_hash.is_empty() {
        return Err(format!(
            "world-server node `{node_id}` has no game_data_projection_hash; heartbeat must pin an active GameData release"
        ));
    }
    let character_release_id = character_release_id.trim();
    if character_release_id != node_release_id {
        return Err(format!(
            "game_data_release_id `{character_release_id}` does not match world-server node `{node_id}` release `{node_release_id}`"
        ));
    }
    let character_hash = character_projection_hash.trim();
    if character_hash != node_hash {
        return Err(format!(
            "game_data_projection_hash `{character_hash}` does not match world-server node `{node_id}` hash `{node_hash}`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_pins_required_when_template_present() {
        validate_release_pins_non_empty("", "abc").expect_err("empty release");
        validate_release_pins_non_empty("release", "").expect_err("empty hash");
        validate_release_pins_non_empty("release", "abc").expect("ok");
    }

    #[test]
    fn world_node_mismatch_rejected() {
        let err = validate_release_against_world_node(
            "release-a",
            "hash-a",
            "release-b",
            "hash-b",
            "node-1",
        )
        .expect_err("release mismatch");
        assert!(err.contains("game_data_release_id"), "{err}");
    }

    #[test]
    fn world_node_hash_mismatch_rejected() {
        let err = validate_release_against_world_node(
            "release-a",
            "hash-a",
            "release-a",
            "hash-b",
            "node-1",
        )
        .expect_err("hash mismatch");
        assert!(err.contains("game_data_projection_hash"), "{err}");
    }

    #[test]
    fn empty_world_node_pins_rejected() {
        let err = validate_release_against_world_node("", "", "release-a", "hash-a", "node-1")
            .expect_err("empty node release");
        assert!(err.contains("game_data_release_id"), "{err}");

        let err =
            validate_release_against_world_node("release-a", "", "release-a", "hash-a", "node-1")
                .expect_err("empty node hash");
        assert!(err.contains("game_data_projection_hash"), "{err}");
    }
}
