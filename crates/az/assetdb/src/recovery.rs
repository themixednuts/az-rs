//! Versioned, read-only recovery adapter for destructive `AssetDB` resets.
//!
//! This module deliberately bypasses Drizzle migrations. A reset must be able
//! to recover the sole non-regenerable fact (unsaved editor payloads) from the
//! Wave 4 schema after the checked-in chain has already become the Wave 5
//! baseline. Normal runtime opens remain migration-owned and fail loudly.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use futures_lite::future::block_on;
use thiserror::Error;
use turso::{Row, Value};

use crate::{Digest, Encoding, RecoveredRoot, RecoveredWorkspace, UnsavedPayload, WorkspaceKey};

const RECOVERY_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum RecoveryExportError {
    #[error("open AssetDB recovery reader: {0}")]
    Open(#[from] az_turso::OpenError),
    #[error("query AssetDB recovery source during {operation}: {source}")]
    Query {
        operation: &'static str,
        #[source]
        source: turso::Error,
    },
    #[error("decode AssetDB recovery row during {operation}: {source}")]
    Decode {
        operation: &'static str,
        #[source]
        source: turso::Error,
    },
    #[error("decode excluded paths for `{path}`: {source}")]
    Exclusions {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported AssetDB schema: expected Wave 5 `payloads` or Wave 4 payload tables")]
    UnsupportedSchema,
    #[error("ambiguous AssetDB schema: Wave 5 and Wave 4 payload tables coexist")]
    AmbiguousSchema,
    #[error("invalid recovery payload `{document}`: {reason}")]
    InvalidPayload { document: String, reason: String },
    #[error("multiple unsaved recovery rows have document identity `{document}` in one workspace")]
    DuplicateDocument { document: String },
}

/// Export every unsaved payload without applying the checked-in migration
/// chain. Both the Wave 5 schema and its immediate Wave 4 predecessor are
/// accepted; any other shape is rejected explicitly.
///
/// # Errors
///
/// Returns an error if the database at `path` cannot be opened for offline
/// maintenance, if its table set matches neither the Wave 5 nor the Wave 4
/// shape, if a row cannot be read or decoded, or if the exported payloads fail
/// [`validate_recovery_payloads`].
pub fn export_unsaved_payloads_for_reset(
    path: impl AsRef<Path>,
) -> Result<Vec<UnsavedPayload>, RecoveryExportError> {
    let path = path.as_ref();
    let text = path.to_string_lossy().into_owned();
    block_on(async move {
        let database =
            az_turso::open_local_for_offline_maintenance(&text, RECOVERY_BUSY_TIMEOUT, false)
                .await?;
        // Scope the connection borrow so the database handle is released before the
        // sort and validation tail, instead of being held for the whole export.
        let mut payloads = {
            let connection = database.connection();
            let tables = schema_tables(connection).await?;
            let wave5 = tables.contains("payloads");
            let authored = tables.contains("authored_documents");
            let sources = tables.contains("source_payloads");
            match (wave5, authored, sources) {
                (true, false, false) => export_wave5(connection).await?,
                (false, true, true) => export_wave4(connection).await?,
                (true, _, _) => return Err(RecoveryExportError::AmbiguousSchema),
                (false, _, _) => return Err(RecoveryExportError::UnsupportedSchema),
            }
        };
        drop(database);
        validate_recovery_payloads(&payloads)?;
        payloads.sort_by(|left, right| {
            (
                &left.workspace.key.project,
                &left.workspace.key.root,
                &left.workspace.key.branch,
                &left.document,
            )
                .cmp(&(
                    &right.workspace.key.project,
                    &right.workspace.key.root,
                    &right.workspace.key.branch,
                    &right.document,
                ))
        });
        Ok(payloads)
    })
}

async fn schema_tables(
    connection: &turso::Connection,
) -> Result<HashSet<String>, RecoveryExportError> {
    let mut rows = connection
        .query(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name IN ('payloads', 'authored_documents', 'source_payloads')",
            (),
        )
        .await
        .map_err(|source| RecoveryExportError::Query {
            operation: "schema detection",
            source,
        })?;
    let mut tables = HashSet::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|source| RecoveryExportError::Query {
            operation: "schema detection",
            source,
        })?
    {
        tables.insert(text(&row, 0, "schema detection")?);
    }
    Ok(tables)
}

async fn export_wave5(
    connection: &turso::Connection,
) -> Result<Vec<UnsavedPayload>, RecoveryExportError> {
    let expected = scalar_count(
        connection,
        "SELECT COUNT(*) FROM payloads WHERE saved IS NULL OR saved != revision",
        "Wave 5 payload count",
    )
    .await?;
    let sql = "SELECT w.project, w.root, w.branch, w.created, w.updated, r.key, wr.owner, wr.path, wr.exclusions, p.path, p.document, p.schema, p.encoding, p.revision, p.saved, p.digest, p.bytes, p.payload, p.checkpoint, p.session, p.project, p.deleted, p.created, p.updated FROM payloads AS p INNER JOIN workspaces AS w ON p.workspace_pk = w.workspace_id INNER JOIN roots AS r ON p.root_pk = r.root_id INNER JOIN workspace_roots AS wr ON wr.workspace_pk = p.workspace_pk AND wr.root_pk = p.root_pk WHERE p.saved IS NULL OR p.saved != p.revision ORDER BY p.payload_id";
    let payloads = collect_rows(connection, sql, "Wave 5 payload export", |row| {
        decode_payload(row, None, false)
    })
    .await?;
    if payloads.len() != expected {
        return Err(RecoveryExportError::InvalidPayload {
            document: "payloads".to_owned(),
            reason: format!(
                "{expected} unsaved rows exist but {} have a complete workspace-root identity",
                payloads.len()
            ),
        });
    }
    Ok(payloads)
}

async fn export_wave4(
    connection: &turso::Connection,
) -> Result<Vec<UnsavedPayload>, RecoveryExportError> {
    let source_count = scalar_count(
        connection,
        "SELECT COUNT(*) FROM source_payloads WHERE saved_revision IS NULL OR saved_revision != revision",
        "Wave 4 byte payload count",
    )
    .await?;
    let source_sql = "SELECT w.project_id, w.workspace_root, w.branch, w.created_unix_ms, w.updated_unix_ms, sf.portable_key, wr.owner_id, wr.source_root, wr.excluded_paths, p.source_path, p.source_path, p.schema_type, p.revision, p.saved_revision, p.content_hash, p.byte_length, p.payload_bytes, p.saved_payload_bytes, p.session_id, p.project_id, p.deleted, p.created_unix_ms, p.updated_unix_ms FROM source_payloads AS p INNER JOIN workspace_views AS w ON p.workspace_view_pk = w.workspace_view_id INNER JOIN scan_folders AS sf ON sf.portable_key = p.source_root_key INNER JOIN workspace_source_roots AS wr ON wr.workspace_view_pk = p.workspace_view_pk AND wr.scan_folder_pk = sf.scan_folder_id WHERE p.saved_revision IS NULL OR p.saved_revision != p.revision ORDER BY p.source_payload_id";
    let mut payloads = collect_rows(
        connection,
        source_sql,
        "Wave 4 byte payload export",
        |row| decode_payload(row, Some(Encoding::Bytes), false),
    )
    .await?;
    if payloads.len() != source_count {
        return Err(RecoveryExportError::InvalidPayload {
            document: "source_payloads".to_owned(),
            reason: format!(
                "{source_count} unsaved rows exist but {} have a complete workspace-root identity",
                payloads.len()
            ),
        });
    }

    // Authored documents predate an explicit root FK. Their only valid owner
    // is the workspace's unique writable root; zero or multiple matches leave
    // the document identity underived and fail the reset preflight.
    let authored_sql = "SELECT w.project_id, w.workspace_root, w.branch, w.created_unix_ms, w.updated_unix_ms, sf.portable_key, wr.owner_id, wr.source_root, wr.excluded_paths, p.source_path, p.document_id, p.schema_type, p.revision, p.saved_revision, p.content_hash, p.byte_length, CAST(p.payload_ron AS BLOB), CAST(p.saved_payload_ron AS BLOB), p.session_id, p.project_id, p.deleted, p.created_unix_ms, p.updated_unix_ms FROM authored_documents AS p INNER JOIN workspace_views AS w ON p.workspace_view_pk = w.workspace_view_id INNER JOIN workspace_source_roots AS wr ON wr.workspace_view_pk = p.workspace_view_pk INNER JOIN scan_folders AS sf ON sf.scan_folder_id = wr.scan_folder_pk AND sf.writable = 1 WHERE p.saved_revision IS NULL OR p.saved_revision != p.revision ORDER BY p.authored_document_id";
    let authored_count = scalar_count(
        connection,
        "SELECT COUNT(*) FROM authored_documents WHERE saved_revision IS NULL OR saved_revision != revision",
        "Wave 4 authored payload count",
    )
    .await?;
    let authored = collect_rows(
        connection,
        authored_sql,
        "Wave 4 authored payload export",
        |row| decode_payload(row, Some(Encoding::Ron), true),
    )
    .await?;
    if authored.len() != authored_count {
        return Err(RecoveryExportError::InvalidPayload {
            document: "authored_documents".to_owned(),
            reason: format!(
                "{authored_count} unsaved rows exist but {} resolve through exactly one writable workspace root",
                authored.len()
            ),
        });
    }
    payloads.extend(authored);
    Ok(payloads)
}

async fn scalar_count(
    connection: &turso::Connection,
    sql: &str,
    operation: &'static str,
) -> Result<usize, RecoveryExportError> {
    let mut rows = connection
        .query(sql, ())
        .await
        .map_err(|source| RecoveryExportError::Query { operation, source })?;
    let row = rows
        .next()
        .await
        .map_err(|source| RecoveryExportError::Query { operation, source })?
        .ok_or_else(|| RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "count query returned no row".to_owned(),
        })?;
    let count = integer(&row, 0, operation)?;
    usize::try_from(count).map_err(|_| RecoveryExportError::InvalidPayload {
        document: operation.to_owned(),
        reason: format!("count {count} is outside usize"),
    })
}

async fn collect_rows<F>(
    connection: &turso::Connection,
    sql: &str,
    operation: &'static str,
    mut decode: F,
) -> Result<Vec<UnsavedPayload>, RecoveryExportError>
where
    F: FnMut(&Row) -> Result<UnsavedPayload, RecoveryExportError>,
{
    let mut rows = connection
        .query(sql, ())
        .await
        .map_err(|source| RecoveryExportError::Query { operation, source })?;
    let mut payloads = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|source| RecoveryExportError::Query { operation, source })?
    {
        payloads.push(decode(&row)?);
    }
    Ok(payloads)
}

fn decode_payload(
    row: &Row,
    legacy_encoding: Option<Encoding>,
    require_document_derivation: bool,
) -> Result<UnsavedPayload, RecoveryExportError> {
    let path = text(row, 9, "payload path")?;
    let document = text(row, 10, "payload document")?;
    if require_document_derivation && document != path {
        return Err(RecoveryExportError::InvalidPayload {
            document,
            reason: format!("document id is not derived from source path `{path}`"),
        });
    }
    let exclusions_json = text(row, 8, "workspace-root exclusions")?;
    let exclusions = serde_json::from_str(&exclusions_json).map_err(|source| {
        RecoveryExportError::Exclusions {
            path: path.clone(),
            source,
        }
    })?;
    let (encoding, revision_index) = if let Some(encoding) = legacy_encoding {
        (encoding, 12)
    } else {
        let encoding = match integer(row, 12, "payload encoding")? {
            0 => Encoding::Ron,
            1 => Encoding::Bytes,
            value => {
                return Err(RecoveryExportError::InvalidPayload {
                    document,
                    reason: format!("invalid encoding value {value}"),
                });
            }
        };
        (encoding, 13)
    };
    Ok(UnsavedPayload {
        workspace: RecoveredWorkspace {
            key: WorkspaceKey {
                project: text(row, 0, "workspace project")?,
                root: text(row, 1, "workspace root")?,
                branch: text(row, 2, "workspace branch")?,
            },
            created: integer(row, 3, "workspace created time")?,
            updated: integer(row, 4, "workspace updated time")?,
        },
        root: RecoveredRoot {
            key: text(row, 5, "root key")?,
            owner: text(row, 6, "root owner")?,
            path: text(row, 7, "root path")?,
            exclusions,
        },
        path,
        document,
        schema: text(row, 11, "payload schema")?,
        encoding,
        revision: integer(row, revision_index, "payload revision")?,
        saved: optional_integer(row, revision_index + 1, "saved revision")?,
        digest: digest(row, revision_index + 2, "payload digest")?,
        bytes: integer(row, revision_index + 3, "payload byte length")?,
        payload: blob(row, revision_index + 4, "payload bytes")?,
        checkpoint: optional_blob(row, revision_index + 5, "payload checkpoint")?,
        session: optional_text(row, revision_index + 6, "payload session")?,
        project: text(row, revision_index + 7, "payload project")?,
        deleted: integer(row, revision_index + 8, "payload deleted flag")? != 0,
        created: integer(row, revision_index + 9, "payload created time")?,
        updated: integer(row, revision_index + 10, "payload updated time")?,
    })
}

/// Validate the natural identity and bytes of a persisted recovery artifact.
///
/// Reset applies this both immediately after the read-only legacy export and
/// when resuming from disk, so the artifact has one semantic trust boundary.
///
/// # Errors
///
/// Returns [`RecoveryExportError::InvalidPayload`] if a payload's path is not
/// project-relative, if its document id is not derived from that path, or if
/// its recorded hash does not match its bytes, and
/// [`RecoveryExportError::DuplicateDocument`] if two payloads share a document
/// identity.
pub fn validate_recovery_payloads(payloads: &[UnsavedPayload]) -> Result<(), RecoveryExportError> {
    let mut documents = HashSet::new();
    for payload in payloads {
        validate_project_relative(&payload.path).map_err(|reason| {
            RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason,
            }
        })?;
        if payload.document != payload.path {
            return Err(RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason: format!(
                    "document id is not derived from source path `{}`",
                    payload.path
                ),
            });
        }
        if payload.project != payload.workspace.key.project {
            return Err(RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason: "payload project does not match its workspace identity".to_owned(),
            });
        }
        if payload.saved == Some(payload.revision) {
            return Err(RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason: "recovery export contains a fully saved payload".to_owned(),
            });
        }
        if payload.bytes != i64::try_from(payload.payload.len()).unwrap_or(i64::MAX) {
            return Err(RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason: "declared byte length does not match payload".to_owned(),
            });
        }
        if Digest::from(blake3::hash(&payload.payload)) != payload.digest {
            return Err(RecoveryExportError::InvalidPayload {
                document: payload.document.clone(),
                reason: "stored digest does not match payload".to_owned(),
            });
        }
        let key = (
            payload.workspace.key.project.clone(),
            payload.workspace.key.root.clone(),
            payload.workspace.key.branch.clone(),
            payload.document.clone(),
        );
        if !documents.insert(key) {
            return Err(RecoveryExportError::DuplicateDocument {
                document: payload.document.clone(),
            });
        }
    }
    Ok(())
}

fn validate_project_relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|component| {
            component.is_empty() || component == "." || component == ".." || component.contains(':')
        })
    {
        return Err(format!(
            "`{path}` is not a normalized project-relative source path"
        ));
    }
    Ok(())
}

fn text(row: &Row, index: usize, operation: &'static str) -> Result<String, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Text(value) => Ok(value),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected text".to_owned(),
        }),
    }
}

fn integer(row: &Row, index: usize, operation: &'static str) -> Result<i64, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Integer(value) => Ok(value),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected integer".to_owned(),
        }),
    }
}

fn optional_integer(
    row: &Row,
    index: usize,
    operation: &'static str,
) -> Result<Option<i64>, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(value)),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected nullable integer".to_owned(),
        }),
    }
}

fn optional_text(
    row: &Row,
    index: usize,
    operation: &'static str,
) -> Result<Option<String>, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Null => Ok(None),
        Value::Text(value) if value.is_empty() => Ok(None),
        Value::Text(value) => Ok(Some(value)),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected nullable text".to_owned(),
        }),
    }
}

fn blob(row: &Row, index: usize, operation: &'static str) -> Result<Vec<u8>, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Blob(value) => Ok(value),
        Value::Text(value) => Ok(value.into_bytes()),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected blob or legacy text".to_owned(),
        }),
    }
}

fn optional_blob(
    row: &Row,
    index: usize,
    operation: &'static str,
) -> Result<Option<Vec<u8>>, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Null => Ok(None),
        Value::Blob(value) => Ok(Some(value)),
        Value::Text(value) => Ok(Some(value.into_bytes())),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected nullable blob or legacy text".to_owned(),
        }),
    }
}

fn digest(row: &Row, index: usize, operation: &'static str) -> Result<Digest, RecoveryExportError> {
    match row
        .get_value(index)
        .map_err(|source| RecoveryExportError::Decode { operation, source })?
    {
        Value::Blob(value) => {
            let bytes: [u8; Digest::BYTE_LENGTH] =
                value
                    .try_into()
                    .map_err(|value: Vec<u8>| RecoveryExportError::InvalidPayload {
                        document: operation.to_owned(),
                        reason: format!("digest has {} bytes", value.len()),
                    })?;
            Ok(Digest::from_bytes(bytes))
        }
        Value::Text(value) => value
            .parse()
            .map_err(|source| RecoveryExportError::InvalidPayload {
                document: operation.to_owned(),
                reason: format!("invalid legacy digest: {source}"),
            }),
        _ => Err(RecoveryExportError::InvalidPayload {
            document: operation.to_owned(),
            reason: "expected digest blob or legacy hexadecimal text".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_fixture(document: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let database = temp.path().join("assetdb.sqlite");
        let path = database.to_string_lossy().into_owned();
        let project_root = temp.path().join("project").to_string_lossy().into_owned();
        let source_root = temp
            .path()
            .join("project/assets")
            .to_string_lossy()
            .into_owned();
        block_on(async {
            let local = az_turso::open_local(&path, RECOVERY_BUSY_TIMEOUT)
                .await
                .expect("open fixture");
            local
                .connection()
                .execute_batch(
                    "CREATE TABLE workspace_views(workspace_view_id INTEGER PRIMARY KEY, project_id TEXT NOT NULL, workspace_root TEXT NOT NULL, branch TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL);
                     CREATE TABLE scan_folders(scan_folder_id INTEGER PRIMARY KEY, portable_key TEXT NOT NULL, writable INTEGER NOT NULL);
                     CREATE TABLE workspace_source_roots(workspace_source_root_id INTEGER PRIMARY KEY, workspace_view_pk INTEGER NOT NULL, scan_folder_pk INTEGER NOT NULL, owner_id TEXT NOT NULL, source_root TEXT NOT NULL, excluded_paths TEXT NOT NULL);
                     CREATE TABLE authored_documents(authored_document_id INTEGER PRIMARY KEY, workspace_view_pk INTEGER NOT NULL, project_id TEXT NOT NULL, session_id TEXT NOT NULL, document_id TEXT NOT NULL, source_path TEXT NOT NULL, schema_type TEXT NOT NULL, revision INTEGER NOT NULL, saved_revision INTEGER, content_hash BLOB NOT NULL, byte_length INTEGER NOT NULL, payload_ron TEXT NOT NULL, saved_payload_ron TEXT, deleted INTEGER NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL);
                     CREATE TABLE source_payloads(source_payload_id INTEGER PRIMARY KEY, workspace_view_pk INTEGER NOT NULL, project_id TEXT NOT NULL, session_id TEXT NOT NULL, source_root_key TEXT NOT NULL, source_path TEXT NOT NULL, schema_type TEXT NOT NULL, revision INTEGER NOT NULL, saved_revision INTEGER, content_hash BLOB NOT NULL, byte_length INTEGER NOT NULL, payload_bytes BLOB NOT NULL, saved_payload_bytes BLOB, deleted INTEGER NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL);",
                )
                .await
                .expect("create legacy schema");
            local
                .connection()
                .execute(
                    "INSERT INTO workspace_views VALUES (1, 'project.test', ?1, 'main', 10, 11)",
                    (project_root,),
                )
                .await
                .unwrap();
            local
                .connection()
                .execute(
                    "INSERT INTO scan_folders VALUES (1, 'project-assets', 1)",
                    (),
                )
                .await
                .unwrap();
            local
                .connection()
                .execute(
                    "INSERT INTO workspace_source_roots VALUES (1, 1, 1, 'project.test', ?1, '[]')",
                    (source_root,),
                )
                .await
                .unwrap();
            let payload = b"(nodes:[])".to_vec();
            let digest = blake3::hash(&payload).as_bytes().to_vec();
            local
                .connection()
                .execute(
                    "INSERT INTO authored_documents VALUES (1, 1, 'project.test', 'session', ?1, 'graphs/test.ron', 'graph', 2, 1, ?2, ?3, ?4, NULL, 0, 12, 13)",
                    (document.to_owned(), digest, i64::try_from(payload.len()).unwrap_or(i64::MAX), String::from_utf8(payload).unwrap()),
                )
                .await
                .unwrap();
            local.checkpoint().await.expect("checkpoint fixture");
            local.flush_to_disk().expect("flush fixture");
        });
        (temp, database)
    }

    #[test]
    fn project_relative_document_guard_rejects_aliases_and_traversal() {
        assert!(validate_project_relative("graphs/logic.visual.ron").is_ok());
        let absolute = std::env::temp_dir().join("graphs/logic.ron");
        let absolute = absolute.to_string_lossy();
        for invalid in [
            "",
            "/graphs/logic.ron",
            "graphs\\logic.ron",
            "graphs//logic.ron",
            "graphs/../logic.ron",
            absolute.as_ref(),
        ] {
            assert!(validate_project_relative(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn wave4_adapter_recovers_unsaved_authored_payload_by_natural_identity() {
        let (_temp, database) = legacy_fixture("graphs/test.ron");
        let payloads = export_unsaved_payloads_for_reset(&database).expect("export legacy state");
        assert_eq!(payloads.len(), 1);
        let payload = &payloads[0];
        assert_eq!(payload.workspace.key.project, "project.test");
        assert_eq!(payload.root.key, "project-assets");
        assert_eq!(payload.document, "graphs/test.ron");
        assert_eq!(payload.encoding, Encoding::Ron);
        assert_eq!(payload.revision, 2);
        assert_eq!(payload.saved, Some(1));
    }

    #[test]
    fn wave4_adapter_fails_when_document_identity_is_not_path_derived() {
        let (_temp, database) = legacy_fixture("opaque-editor-id");
        let error = export_unsaved_payloads_for_reset(&database)
            .expect_err("non-derived document id must stop reset");
        assert!(
            matches!(error, RecoveryExportError::InvalidPayload { .. }),
            "{error}"
        );
    }

    #[test]
    fn persisted_recovery_validation_rejects_payload_tampering() {
        let (_temp, database) = legacy_fixture("graphs/test.ron");
        let mut payloads =
            export_unsaved_payloads_for_reset(&database).expect("export legacy state");
        payloads[0].payload.push(0);

        let error = validate_recovery_payloads(&payloads)
            .expect_err("artifact bytes must match their declared digest and length");
        assert!(
            matches!(error, RecoveryExportError::InvalidPayload { .. }),
            "{error}"
        );
    }

    #[test]
    fn recovery_rejects_mixed_schema_generations() {
        let (_temp, database) = legacy_fixture("graphs/test.ron");
        let path = database.to_string_lossy().into_owned();
        block_on(async {
            let local =
                az_turso::open_local_for_offline_maintenance(&path, RECOVERY_BUSY_TIMEOUT, false)
                    .await
                    .expect("open fixture for partial migration");
            local
                .connection()
                .execute("CREATE TABLE payloads(payload_id INTEGER PRIMARY KEY)", ())
                .await
                .expect("add conflicting generation marker");
            local.checkpoint().await.expect("checkpoint fixture");
            local.flush_to_disk().expect("flush fixture");
        });

        assert!(matches!(
            export_unsaved_payloads_for_reset(&database),
            Err(RecoveryExportError::AmbiguousSchema)
        ));
    }
}
