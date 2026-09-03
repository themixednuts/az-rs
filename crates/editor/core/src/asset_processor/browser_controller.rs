// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone)]
pub struct EditorAssetBrowserController {
    #[cfg(test)]
    client: AssetProcessorClient,
    session_id: String,
    session_uuid: Option<Uuid>,
    session_slug: Option<String>,
    workspace_identity: Option<AssetBrowserWorkspaceIdentity>,
    supervisor_descriptor: Option<ServiceDescriptor>,
    #[cfg(test)]
    supervisor: Option<SessionSupervisorClient>,
}

#[derive(Clone, Debug)]
struct AssetBrowserWorkspaceIdentity {
    workspace: AttachedWorkspace,
    workspace_id: i64,
}

impl AssetBrowserWorkspaceIdentity {
    fn from_attach_session(session: &EditorAttachSession) -> Self {
        Self {
            workspace: session.workspace.clone(),
            workspace_id: session.workspace_snapshot.workspace_id,
        }
    }
}

impl EditorAssetBrowserController {
    #[cfg(test)]
    #[must_use]
    pub fn new(client: AssetProcessorClient, session_id: impl Into<String>) -> Self {
        Self {
            client,
            session_id: session_id.into(),
            session_uuid: None,
            session_slug: None,
            workspace_identity: None,
            supervisor_descriptor: None,
            supervisor: None,
        }
    }

    /// Binds a browser controller to an attached editor session, remembering
    /// the session's workspace identity and supervisor descriptor.
    ///
    /// # Errors
    ///
    /// Binding only records identity, so this returns `Ok` outside tests; the
    /// asset-processor descriptor is resolved lazily on the first read. The
    /// test build connects eagerly and so returns any error
    /// [`AssetProcessorClient::connect_for_session`] returns —
    /// [`EditorError::ServiceDiscovery`] for a descriptor that fails its
    /// identity, protocol-version, or brokered-template checks, and
    /// [`EditorError::RpcTransport`] for an endpoint that cannot be reached.
    // Only the `cfg(test)` build awaits `connect_for_session` here; dropping
    // `async` would break that arm and every caller that awaits this.
    #[cfg_attr(not(test), allow(clippy::unused_async))]
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        #[cfg(test)]
        let client = AssetProcessorClient::connect_for_session(
            &session.services.asset_processor,
            session.session_id,
        )
        .await?;
        Ok(Self {
            #[cfg(test)]
            client,
            session_id: session.session_id.to_string(),
            session_uuid: Some(session.session_id),
            session_slug: Some(session.session_slug.clone()),
            workspace_identity: Some(AssetBrowserWorkspaceIdentity::from_attach_session(session)),
            supervisor_descriptor: Some(session.session_supervisor.clone()),
            #[cfg(test)]
            supervisor: None,
        })
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_session_supervisor(
        mut self,
        supervisor: SessionSupervisorClient,
        session_slug: impl Into<String>,
        session_uuid: Uuid,
    ) -> Self {
        self.supervisor = Some(supervisor);
        self.session_slug = Some(session_slug.into());
        self.session_uuid = Some(session_uuid);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_attached_workspace_identity(
        mut self,
        project_id: impl Into<String>,
        workspace_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        workspace_id: i64,
    ) -> Self {
        self.workspace_identity = Some(AssetBrowserWorkspaceIdentity {
            workspace: AttachedWorkspace {
                project_id: project_id.into(),
                workspace_root: workspace_path.into().to_string_lossy().into_owned(),
                branch: branch.into(),
            },
            workspace_id,
        });
        self
    }

    /// Re-resolves the asset-processor descriptor through session-supervisor
    /// and returns a controller bound to the current endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if this controller has no
    /// attached session slug, session id, or supervisor descriptor; if
    /// session-supervisor reports a session whose id, slug, project, or
    /// workspace root does not match the attached identity; or if the
    /// descriptor supervisor resolves no longer has the same connection
    /// contract as the one the session manifest advertises. Returns
    /// [`EditorError::MissingSessionService`] if that manifest lists no
    /// asset-processor service. Also returns any error connecting to
    /// session-supervisor or asset-processor produces, including
    /// [`EditorError::RpcTransport`] for an unreachable endpoint and
    /// [`EditorError::ServiceProtocol`] for a status call that fails in flight.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn refresh_client_from_supervisor(&self) -> EditorResult<Self> {
        #[cfg(test)]
        if self.supervisor.is_none() && self.supervisor_descriptor.is_none() {
            return Ok(self.clone());
        }
        let Some(session_slug) = &self.session_slug else {
            #[cfg(test)]
            return Ok(self.clone());

            #[cfg(not(test))]
            return Err(EditorError::ServiceDiscovery(
                "asset browser refresh requires an attached session slug".to_string(),
            ));
        };

        let Some(session_id) = self.session_uuid else {
            #[cfg(test)]
            return Ok(self.clone());

            #[cfg(not(test))]
            return Err(EditorError::ServiceDiscovery(
                "asset browser refresh requires an attached session id".to_string(),
            ));
        };
        let supervisor = self
            .session_supervisor_client("asset browser refresh")
            .await?;
        let descriptor = self
            .asset_processor_descriptor_from_supervisor(&supervisor, session_slug)
            .await?;
        #[cfg(test)]
        let client = AssetProcessorClient::connect_for_session(&descriptor, session_id).await?;
        #[cfg(not(test))]
        let _ = AssetProcessorClient::connect_for_session(&descriptor, session_id).await?;
        Ok(Self {
            #[cfg(test)]
            client,
            session_id: self.session_id.clone(),
            session_uuid: self.session_uuid,
            session_slug: self.session_slug.clone(),
            workspace_identity: self.workspace_identity.clone(),
            supervisor_descriptor: self.supervisor_descriptor.clone(),
            #[cfg(test)]
            supervisor: self.supervisor.clone(),
        })
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn attached_workspace_snapshot_with_client(
        &self,
        client: AssetProcessorClient,
    ) -> EditorResult<WorkspaceSnapshot> {
        let identity = self.workspace_identity.as_ref().ok_or_else(|| {
            EditorError::ServiceDiscovery(
                "asset browser requires a resolved workspace identity".to_string(),
            )
        })?;
        let snapshot = client
            .workspace_snapshot(AssetRootScope::All)
            .await?
            .ok_or_else(|| {
                EditorError::ServiceDiscovery(format!(
                    "asset-processor has no workspace snapshot for attached workspace `{}` on branch `{}`",
                    identity.workspace.workspace_root, identity.workspace.branch
                ))
            })?;
        self.ensure_workspace_snapshot_matches_attached_identity(&snapshot)?;
        Ok(snapshot)
    }

    fn ensure_workspace_snapshot_matches_attached_identity(
        &self,
        snapshot: &WorkspaceSnapshot,
    ) -> EditorResult<()> {
        if snapshot.roots.is_empty() {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} for session `{}` has no roots",
                snapshot.workspace_id, self.session_id
            )));
        }
        let Some(identity) = &self.workspace_identity else {
            return Ok(());
        };
        if snapshot.workspace_id != identity.workspace_id {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} does not match attached workspace {}",
                snapshot.workspace_id, identity.workspace_id
            )));
        }
        if snapshot.project_id != identity.workspace.project_id {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} project `{}` does not match attached project `{}`",
                snapshot.workspace_id, snapshot.project_id, identity.workspace.project_id
            )));
        }
        if !same_protocol_path(
            Path::new(&snapshot.workspace_root),
            Path::new(&identity.workspace.workspace_root),
        ) {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} root `{}` does not match attached workspace `{}`",
                snapshot.workspace_id, snapshot.workspace_root, identity.workspace.workspace_root
            )));
        }
        if snapshot.branch != identity.workspace.branch {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} branch `{}` does not match attached branch `{}`",
                snapshot.workspace_id, snapshot.branch, identity.workspace.branch
            )));
        }

        let mut seen_workspace_root_ids = HashSet::new();
        let mut seen_portable_keys = HashSet::new();
        for root in &snapshot.roots {
            ensure_asset_browser_root(snapshot, root)?;
            if !seen_workspace_root_ids.insert(root.workspace_root_id) {
                return Err(EditorError::ServiceDiscovery(format!(
                    "asset-processor workspace snapshot {} returned duplicate workspace-root id {}",
                    snapshot.workspace_id, root.workspace_root_id
                )));
            }
            if !seen_portable_keys.insert(root.portable_key.as_str()) {
                return Err(EditorError::ServiceDiscovery(format!(
                    "asset-processor workspace snapshot {} returned duplicate root key `{}`",
                    snapshot.workspace_id, root.portable_key
                )));
            }
        }
        Ok(())
    }

    /// Loads the first page of browser entries together with the workspace
    /// roots to show above them.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, if this controller holds no workspace identity, if
    /// asset-processor has no workspace snapshot, or if that snapshot has no
    /// roots or does not match the attached workspace id, project, root, or
    /// branch. Otherwise returns any error
    /// [`AssetProcessorClient::workspace_snapshot`] or
    /// [`AssetProcessorClient::workspace_entry_page`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_first_page(&self) -> EditorResult<EditorAssetBrowserStatus> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        let snapshot = self
            .attached_workspace_snapshot_with_client(client.clone())
            .await?;
        self.load_page_with_client(client, None, workspace_roots_to_ui(&snapshot))
            .await
    }

    /// Loads the page of browser entries after `after_entry_id`, without
    /// republishing the workspace roots.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::load_first_page`]: the
    /// [`EditorError::ServiceDiscovery`] cases for an unresolvable client,
    /// missing workspace identity, or a snapshot that is absent or does not
    /// match the attached workspace, plus any error
    /// [`AssetProcessorClient::workspace_entry_page`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_page(
        &self,
        after_entry_id: Option<i64>,
    ) -> EditorResult<EditorAssetBrowserStatus> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        self.attached_workspace_snapshot_with_client(client.clone())
            .await?;
        self.load_page_with_client(client, after_entry_id, Vec::new())
            .await
    }

    /// Loads the builder catalog and projects it into UI data.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::builder_catalog`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_builder_catalog(&self) -> EditorResult<EditorAssetBuilderCatalog> {
        let catalog = self
            .asset_processor_client()
            .await?
            .builder_catalog()
            .await?;
        Ok(asset_builder_catalog_to_ui(catalog))
    }

    /// Reads asset-processor health and projects it into UI activity data.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::health`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn health(&self) -> EditorResult<EditorAssetProcessorActivity> {
        let health = self.asset_processor_client().await?.health().await?;
        Ok(asset_processor_activity_to_ui(&self.session_id, health))
    }

    /// Loads catalog products for a platform, normalizing a blank or `all`
    /// platform to the default.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, if this controller holds no workspace identity, or
    /// if the workspace snapshot is absent or does not match the attached
    /// workspace, and otherwise any error
    /// [`AssetProcessorClient::catalog_products`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_catalog_products(
        &self,
        platform: impl Into<String>,
    ) -> EditorResult<EditorCatalogProductsStatus> {
        let requested: String = platform.into();
        let platform = normalize_catalog_product_platform(&requested);
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        self.attached_workspace_snapshot_with_client(client.clone())
            .await?;
        let result = client.catalog_products(platform.clone()).await?;
        Ok(catalog_products_to_ui(
            self.session_id.clone(),
            platform,
            result,
        ))
    }

    /// Creates a source file from a default template under the attached
    /// session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::create_source_file_from_default_template`]
    /// returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_source_file_from_default_template(
        &self,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
        schema_type: impl Into<String>,
    ) -> EditorResult<SourceFileCreateResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .create_source_file_from_default_template(
                self.session_id.clone(),
                source_root.into(),
                source_path.into(),
                schema_type.into(),
            )
            .await
    }

    /// Lists what depends on one source file under the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::source_dependents_by_session`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn source_dependents(
        &self,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<SourceDependentsResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .source_dependents_by_session(
                self.session_id.clone(),
                source_root.into(),
                source_path.into(),
            )
            .await
    }

    /// Requeues every builder job for one source file under the attached
    /// session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::force_reprocess_asset_by_session`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn force_reprocess_asset(
        &self,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<ForceReprocessAssetResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .force_reprocess_asset_by_session(
                self.session_id.clone(),
                source_root.into(),
                source_path.into(),
            )
            .await
    }

    /// Rescans the asset roots in `root_scope` under the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::reconcile_asset_sources_by_session`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn reconcile_asset_sources(
        &self,
        root_scope: AssetRootScope,
    ) -> EditorResult<ReconcileAssetSourcesResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .reconcile_asset_sources_by_session(self.session_id.clone(), root_scope)
            .await
    }

    /// Deletes one source file under the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::delete_source_file`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn delete_source_file(
        &self,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<SourceFileDeleteResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .delete_source_file(
                self.session_id.clone(),
                source_root.into(),
                source_path.into(),
            )
            .await
    }

    /// Moves one source file under the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, and otherwise any error
    /// [`AssetProcessorClient::move_source_file`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn move_source_file(
        &self,
        source_root: impl Into<String>,
        from_source_path: impl Into<String>,
        to_source_path: impl Into<String>,
    ) -> EditorResult<SourceFileMoveResult> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        client
            .move_source_file(
                self.session_id.clone(),
                source_root.into(),
                from_source_path.into(),
                to_source_path.into(),
            )
            .await
    }

    /// Inspects the job `selector` names and projects it into UI data.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the asset-processor client
    /// cannot be resolved, if this controller holds no workspace identity, if
    /// the workspace snapshot is absent or does not match the attached
    /// workspace, or if asset-processor has no job matching `selector`, and
    /// otherwise any error [`AssetProcessorClient::inspect_job`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn inspect_job(
        &self,
        selector: InspectJobSelector,
    ) -> EditorResult<EditorJobInspection> {
        let client = self.scope_asset_processor_client(self.asset_processor_client().await?);
        self.attached_workspace_snapshot_with_client(client.clone())
            .await?;
        let inspection = client.inspect_job(selector.clone()).await?.ok_or_else(|| {
            EditorError::ServiceDiscovery(format!(
                "asset-processor has no matching job inspection for `{selector:?}`"
            ))
        })?;
        Ok(job_inspection_to_ui(inspection))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn load_page_with_client(
        &self,
        client: AssetProcessorClient,
        after_entry_id: Option<i64>,
        roots: Vec<WorkspaceRootData>,
    ) -> EditorResult<EditorAssetBrowserStatus> {
        let result = client
            .workspace_entry_page(
                AssetRootScope::BrowserAssets,
                after_entry_id,
                ASSET_BROWSER_PAGE_SIZE,
            )
            .await?;
        Ok(workspace_entry_page_to_ui(&self.session_id, roots, result))
    }

    const fn scope_asset_processor_client(
        &self,
        client: AssetProcessorClient,
    ) -> AssetProcessorClient {
        match self.session_uuid {
            Some(session_id) => client.with_session_scope(session_id),
            None => client,
        }
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn asset_processor_client(&self) -> EditorResult<AssetProcessorClient> {
        #[cfg(test)]
        if self.supervisor.is_none() && self.supervisor_descriptor.is_none() {
            return Ok(self.client.clone());
        }
        let Some(session_slug) = &self.session_slug else {
            #[cfg(test)]
            return Ok(self.client.clone());

            #[cfg(not(test))]
            return Err(EditorError::ServiceDiscovery(
                "asset browser reads require an attached session slug".to_string(),
            ));
        };

        let Some(session_id) = self.session_uuid else {
            return Err(EditorError::ServiceDiscovery(
                "asset browser reads require an attached session id".to_string(),
            ));
        };
        let supervisor = self
            .session_supervisor_client("asset browser reads")
            .await?;
        let descriptor = self
            .asset_processor_descriptor_from_supervisor(&supervisor, session_slug)
            .await?;
        AssetProcessorClient::connect_for_session(&descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn session_supervisor_client(
        &self,
        operation: &'static str,
    ) -> EditorResult<SessionSupervisorClient> {
        #[cfg(test)]
        if let Some(supervisor) = &self.supervisor {
            return Ok(supervisor.clone());
        }

        let Some(descriptor) = &self.supervisor_descriptor else {
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires a session-supervisor descriptor to resolve the current asset-processor descriptor"
            )));
        };
        let Some(session_id) = self.session_uuid else {
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires an attached session id"
            )));
        };
        SessionSupervisorClient::connect_for_session(descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn asset_processor_descriptor_from_supervisor(
        &self,
        supervisor: &SessionSupervisorClient,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        let status = supervisor.status(session_slug).await?;
        self.ensure_status_matches_attached_identity(&status)?;
        let expected = asset_processor_descriptor_from_status(&status.manifest)?;
        let resolved = supervisor
            .resolve_asset_processor_descriptor(session_slug)
            .await?;
        if !resolved.has_same_connection_contract(&expected) {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor connection contract changed while resolving asset browser refresh for session `{session_slug}`"
            )));
        }
        Ok(resolved)
    }

    fn ensure_status_matches_attached_identity(
        &self,
        status: &ProtoSessionWorkspaceStatus,
    ) -> EditorResult<()> {
        let manifest = &status.manifest;
        if let Some(session_id) = self.session_uuid
            && manifest.id != session_id
        {
            return Err(asset_browser_session_mismatch(
                &manifest.slug,
                &format!(
                    "status session id `{}` does not match attached session id `{}`",
                    manifest.id, session_id
                ),
            ));
        }
        if let Some(session_slug) = &self.session_slug
            && manifest.slug != *session_slug
        {
            return Err(asset_browser_session_mismatch(
                &manifest.slug,
                &format!(
                    "status session slug `{}` does not match attached session slug `{}`",
                    manifest.slug, session_slug
                ),
            ));
        }
        let Some(identity) = &self.workspace_identity else {
            return Ok(());
        };
        if manifest.project_id != identity.workspace.project_id {
            return Err(asset_browser_session_mismatch(
                &manifest.slug,
                &format!(
                    "status project `{}` does not match attached project `{}`",
                    manifest.project_id, identity.workspace.project_id
                ),
            ));
        }
        if !same_protocol_path(
            Path::new(&manifest.workspace_root),
            Path::new(&identity.workspace.workspace_root),
        ) {
            return Err(asset_browser_session_mismatch(
                &manifest.slug,
                &format!(
                    "status workspace `{}` does not match attached workspace `{}`",
                    manifest.workspace_root, identity.workspace.workspace_root
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) fn asset_processor_descriptor_from_status(
    manifest: &ProtoSessionManifest,
) -> EditorResult<ServiceDescriptor> {
    manifest
        .services
        .iter()
        .find(|descriptor| {
            descriptor.id == asset_processor_service_id()
                && descriptor.role == ServiceRole::AssetProcessor
        })
        .ok_or_else(|| EditorError::MissingSessionService {
            session: manifest.slug.clone(),
            service: service_label(&asset_processor_service_id()),
        })
        .cloned()
}
