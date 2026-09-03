// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone)]
pub struct AssetProcessorClient {
    client: asset_capnp::asset_processor::Client,
    editor_service: ServiceId,
    session_id: Option<Uuid>,
    capability_templates: Vec<Capability>,
    descriptor_capabilities_required: bool,
}

pub(crate) struct AssetProcessorEventStreamClient {
    client: AssetProcessorClient,
    connection: az_rpc::ScopedTwopartyClient<asset_capnp::asset_processor::Client>,
}

impl AssetProcessorClient {
    #[cfg(test)]
    #[must_use]
    pub fn new(client: asset_capnp::asset_processor::Client) -> Self {
        Self {
            client,
            editor_service: ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            session_id: None,
            capability_templates: Vec::new(),
            descriptor_capabilities_required: false,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn with_editor_service(
        client: asset_capnp::asset_processor::Client,
        editor_service: ServiceId,
    ) -> Self {
        Self {
            client,
            editor_service,
            session_id: None,
            capability_templates: Vec::new(),
            descriptor_capabilities_required: false,
        }
    }

    #[must_use]
    pub(crate) const fn with_session_scope(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_capability_templates(mut self, capabilities: Vec<Capability>) -> Self {
        self.capability_templates = capabilities;
        self.descriptor_capabilities_required = true;
        self
    }

    /// Connects to an asset-processor descriptor and adopts its brokered
    /// capability templates.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if `descriptor` does not name
    /// the asset-processor service in the [`ServiceRole::AssetProcessor`] role,
    /// advertises another protocol version, or carries capability templates
    /// that are not validly brokered, or [`EditorError::RpcTransport`] if the
    /// descriptor endpoint cannot be reached.
    #[cfg(test)]
    pub async fn connect(descriptor: &ServiceDescriptor) -> EditorResult<Self> {
        validate_asset_processor_descriptor(descriptor)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(Self::from_descriptor_client(
            client,
            descriptor.capabilities.clone(),
        ))
    }

    /// Connects to an asset-processor descriptor and tags this client with the
    /// attached editor session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if `descriptor` fails the
    /// asset-processor identity, protocol-version, or brokered-template checks,
    /// or [`EditorError::RpcTransport`] if the descriptor endpoint cannot be
    /// reached.
    pub async fn connect_for_session(
        descriptor: &ServiceDescriptor,
        session_id: Uuid,
    ) -> EditorResult<Self> {
        validate_asset_processor_descriptor_for_session(descriptor, session_id)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(
            Self::from_descriptor_client(client, descriptor.capabilities.clone())
                .with_session_scope(session_id),
        )
    }

    pub(crate) async fn connect_event_stream_for_session(
        descriptor: &ServiceDescriptor,
        session_id: Uuid,
    ) -> EditorResult<AssetProcessorEventStreamClient> {
        validate_asset_processor_descriptor_for_session(descriptor, session_id)?;
        let connection: az_rpc::ScopedTwopartyClient<asset_capnp::asset_processor::Client> =
            az_rpc::connect_twoparty_bootstrap_scoped(&descriptor.endpoint).await?;
        let client = Self::from_descriptor_client(
            connection.client().clone(),
            descriptor.capabilities.clone(),
        )
        .with_session_scope(session_id);
        Ok(AssetProcessorEventStreamClient { client, connection })
    }

    #[must_use]
    pub const fn editor_service(&self) -> &ServiceId {
        &self.editor_service
    }

    /// Inspects one asset job, or the attempt `selector` names.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as an inspection
    /// result; and [`EditorError::ServiceAuthorityMismatch`] if the returned
    /// inspection names another job or attempt, carries non-positive job or
    /// workspace ids, a nil source guid, or products and dependencies that
    /// belong to another job.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn inspect_job(
        &self,
        selector: InspectJobSelector,
    ) -> EditorResult<Option<JobInspection>> {
        let expected_selector = selector.clone();
        let mut request = self.client.inspect_job_request();
        (InspectJobRequest {
            capability: self.editor_read_capability()?,
            selector,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = InspectJobResult::from_capnp(response.get()?.get_result()?)?;
        if let Some(inspection) = &result.inspection {
            ensure_job_inspection_matches_selector(inspection, &expected_selector, "inspectJob")?;
        }
        Ok(result.inspection)
    }

    /// Reads the catalog of registered asset builders and source schemas.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a catalog result;
    /// and [`EditorError::ServiceAuthorityMismatch`] if that catalog repeats a
    /// builder guid or otherwise fails its identity checks.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn builder_catalog(&self) -> EditorResult<AssetBuilderCatalogResult> {
        let mut request = self.client.builder_catalog_request();
        (AssetBuilderCatalogRequest {
            capability: self.editor_read_capability()?,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = AssetBuilderCatalogResult::from_capnp(response.get()?.get_result()?)?;
        ensure_asset_builder_catalog_result_matches_request(&result)?;
        Ok(result)
    }

    /// Reads asset-processor health, rejecting a peer that is not the expected
    /// service.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] if the call fails in flight or
    /// the reply does not decode as a health report, or
    /// [`EditorError::ServiceDiscovery`] if that report advertises another
    /// protocol version, a role other than [`ServiceRole::AssetProcessor`], or
    /// a service id other than the asset-processor's. This call carries no
    /// capability, so it never fails the capability check.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn health(&self) -> EditorResult<ServiceHealth> {
        let request = self.client.health_request();
        let response = request.send().promise.await?;
        let health = ServiceHealth::from_capnp(response.get()?.get_health()?)?;
        health
            .require_protocol_version(ProtocolVersion::CURRENT)
            .map_err(|error| {
                EditorError::ServiceDiscovery(format!(
                    "asset-processor unavailable until restarted: {error}"
                ))
            })?;
        if health.role != ServiceRole::AssetProcessor {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor health reported role {:?}",
                health.role
            )));
        }
        if health.service.namespace != ASSET_PROCESSOR_NAMESPACE
            || health.service.name != ASSET_PROCESSOR_SERVICE_NAME
        {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor health reported service `{}`/`{}`",
                health.service.namespace, health.service.name
            )));
        }
        Ok(health)
    }

    /// Subscribes an event sink that forwards asset-processor events to `tx`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`], or if asset-processor answers with a
    /// result that declines the subscription, and
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a subscription
    /// result.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn subscribe_events(
        &self,
        tx: Sender<AssetProcessorEvent>,
    ) -> EditorResult<AssetProcessorEventSubscriptionResult> {
        let sink = EditorAssetProcessorEventSink::new(tx).into_client();
        let mut request = self.client.subscribe_events_request();
        {
            let mut params = request.get();
            (AssetProcessorEventSubscriptionRequest {
                capability: self.editor_read_capability()?,
            })
            .to_capnp(params.reborrow().init_request())?;
            params.set_sink(sink);
        }

        let response = request.send().promise.await?;
        let result =
            AssetProcessorEventSubscriptionResult::from_capnp(response.get()?.get_result()?)?;
        if !result.subscribed {
            return Err(EditorError::ServiceDiscovery(
                "asset-processor declined event subscription".to_string(),
            ));
        }
        Ok(result)
    }

    /// Creates a source file from the default template registered for
    /// `schema_type`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_WRITE_PERMISSION`];
    /// [`EditorError::InvalidArgument`] if the system clock is before the Unix
    /// epoch or its millisecond count does not fit in an `i64`;
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a create result;
    /// and [`EditorError::ServiceAuthorityMismatch`] if that result names
    /// another source path or schema type, carries a nil asset guid or
    /// non-positive entry, workspace, or root id, or reports a content hash
    /// that is not 64 hex characters.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_source_file_from_default_template(
        &self,
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
        schema_type: impl Into<String>,
    ) -> EditorResult<SourceFileCreateResult> {
        let source_root = source_root.into();
        let source_path = source_path.into();
        let schema_type = schema_type.into();
        let mut request = self.client.create_source_file_request();
        (SourceFileCreateRequest {
            capability: self.editor_asset_write_capability()?,
            session_id: session_id.into(),
            source_root: source_root.clone(),
            source_path: source_path.clone(),
            schema_type: schema_type.clone(),
            changed_unix_ms: current_unix_ms_i64()?,
            content: SourceFileCreateContent::DefaultTemplate,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = SourceFileCreateResult::from_capnp(response.get()?.get_result()?)?;
        ensure_source_file_create_result_matches_request(&result, &source_path, &schema_type)?;
        Ok(result)
    }

    /// Lists the sources and jobs that depend on one source file.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a dependents
    /// result; and [`EditorError::ServiceAuthorityMismatch`] if that result
    /// answers for another source path, or lists a dependent whose source or
    /// product path is not asset-relative, whose edge, job, or attempt ids are
    /// non-positive, whose job identity fields are blank, or whose build owner
    /// guid is nil.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn source_dependents_by_session(
        &self,
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<SourceDependentsResult> {
        let session_id = session_id.into();
        let source_root = source_root.into();
        let source_path = source_path.into();
        let mut request = self.client.source_dependents_request();
        (SourceDependentsRequest {
            capability: self.editor_read_capability()?,
            session_id,
            source_root: source_root.clone(),
            source_path: source_path.clone(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = SourceDependentsResult::from_capnp(response.get()?.get_result()?)?;
        ensure_source_dependents_result_matches_request(&result, &source_path)?;
        Ok(result)
    }

    /// Requeues every builder job for one source file.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_WRITE_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a reprocess
    /// result; and [`EditorError::ServiceAuthorityMismatch`] if that result
    /// names another source path, carries a nil asset guid, or reports that no
    /// jobs were queued.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn force_reprocess_asset_by_session(
        &self,
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<ForceReprocessAssetResult> {
        let session_id = session_id.into();
        let source_root = source_root.into();
        let source_path = source_path.into();
        let mut request = self.client.force_reprocess_asset_request();
        (ForceReprocessAssetRequest {
            capability: self.editor_asset_write_capability()?,
            session_id,
            source_root: source_root.clone(),
            source_path: source_path.clone(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ForceReprocessAssetResult::from_capnp(response.get()?.get_result()?)?;
        ensure_force_reprocess_asset_result_matches_request(&result, &source_path)?;
        Ok(result)
    }

    /// Rescans the asset roots in `root_scope` against the workspace database.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_WRITE_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a reconcile
    /// result. The result itself is returned unvalidated.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn reconcile_asset_sources_by_session(
        &self,
        session_id: impl Into<String>,
        root_scope: AssetRootScope,
    ) -> EditorResult<ReconcileAssetSourcesResult> {
        let session_id = session_id.into();
        let mut request = self.client.reconcile_asset_sources_request();
        (ReconcileAssetSourcesRequest {
            capability: self.editor_asset_write_capability()?,
            session_id,
            root_scope,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ReconcileAssetSourcesResult::from_capnp(response.get()?.get_result()?)?;
        Ok(result)
    }

    /// Deletes one source file and its workspace entry.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_WRITE_PERMISSION`];
    /// [`EditorError::InvalidArgument`] if the system clock is before the Unix
    /// epoch or its millisecond count does not fit in an `i64`;
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a delete result;
    /// and [`EditorError::ServiceAuthorityMismatch`] if that result names
    /// another source path or does not report the entry as deleted.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn delete_source_file(
        &self,
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> EditorResult<SourceFileDeleteResult> {
        let source_root = source_root.into();
        let source_path = source_path.into();
        let mut request = self.client.delete_source_file_request();
        (SourceFileDeleteRequest {
            capability: self.editor_asset_write_capability()?,
            session_id: session_id.into(),
            source_root,
            source_path: source_path.clone(),
            changed_unix_ms: current_unix_ms_i64()?,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = SourceFileDeleteResult::from_capnp(response.get()?.get_result()?)?;
        ensure_source_file_delete_result_matches_request(&result, &source_path)?;
        Ok(result)
    }

    /// Moves one source file to a new path inside the same root.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_WRITE_PERMISSION`];
    /// [`EditorError::InvalidArgument`] if the system clock is before the Unix
    /// epoch or its millisecond count does not fit in an `i64`;
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a move result; and
    /// [`EditorError::ServiceAuthorityMismatch`] if that result names another
    /// old or new source path, or carries a nil asset guid.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn move_source_file(
        &self,
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        from_source_path: impl Into<String>,
        to_source_path: impl Into<String>,
    ) -> EditorResult<SourceFileMoveResult> {
        let source_root = source_root.into();
        let from_source_path = from_source_path.into();
        let to_source_path = to_source_path.into();
        let mut request = self.client.move_source_file_request();
        (SourceFileMoveRequest {
            capability: self.editor_asset_write_capability()?,
            session_id: session_id.into(),
            source_root,
            from_source_path: from_source_path.clone(),
            to_source_path: to_source_path.clone(),
            changed_unix_ms: current_unix_ms_i64()?,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = SourceFileMoveResult::from_capnp(response.get()?.get_result()?)?;
        ensure_source_file_move_result_matches_request(
            &result,
            &from_source_path,
            &to_source_path,
        )?;
        Ok(result)
    }

    /// Reads the workspace snapshot covering the roots in `root_scope`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a snapshot result;
    /// and [`EditorError::ServiceAuthorityMismatch`] if a returned snapshot
    /// carries a non-positive workspace id, blank identity fields, timestamps
    /// that are negative or out of order, no roots, or roots whose ids or
    /// portable keys repeat.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn workspace_snapshot(
        &self,
        root_scope: AssetRootScope,
    ) -> EditorResult<Option<WorkspaceSnapshot>> {
        let mut request = self.client.workspace_snapshot_request();
        (WorkspaceSnapshotRequest {
            capability: self.editor_read_capability()?,
            root_scope,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = WorkspaceSnapshotResult::from_capnp(response.get()?.get_result()?)?;
        if let Some(snapshot) = &result.snapshot {
            ensure_workspace_snapshot_identity(snapshot)?;
        }
        Ok(result.snapshot)
    }

    /// Reads one page of workspace entries after the `after_entry_id` cursor.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as an entry page; and
    /// [`EditorError::ServiceAuthorityMismatch`] if that page holds more than
    /// `page_size` entries, returns an entry at or before the cursor, repeats
    /// an entry id, fails an entry's identity checks, or carries a next-page
    /// cursor on an empty page or one that does not name the page's last entry.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn workspace_entry_page(
        &self,
        root_scope: AssetRootScope,
        after_entry_id: Option<i64>,
        page_size: u32,
    ) -> EditorResult<WorkspaceEntryPageResult> {
        let mut request = self.client.workspace_entry_page_request();
        (WorkspaceEntryPageRequest {
            capability: self.editor_read_capability()?,
            root_scope,
            after_entry_id,
            page_size,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = WorkspaceEntryPageResult::from_capnp(response.get()?.get_result()?)?;
        ensure_workspace_entry_page_result_matches_request(&result, after_entry_id, page_size)?;
        Ok(result)
    }

    /// Reads the catalog of built products for one platform.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`ASSET_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a catalog result;
    /// and [`EditorError::ServiceAuthorityMismatch`] if that catalog lists a
    /// product built for another platform, repeats a product id, or returns
    /// products out of their product-path, guid, sub-id, product-id order.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn catalog_products(
        &self,
        platform: impl Into<String>,
    ) -> EditorResult<CatalogProductsResult> {
        let platform = platform.into();
        let expected_platform = platform.clone();
        let mut request = self.client.catalog_products_request();
        (CatalogProductsRequest {
            capability: self.editor_read_capability()?,
            platform,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = CatalogProductsResult::from_capnp(response.get()?.get_result()?)?;
        ensure_catalog_products_result_matches_request(&result, &expected_platform)?;
        Ok(result)
    }

    pub(crate) fn editor_read_capability(&self) -> EditorResult<Capability> {
        self.editor_capability(&[ASSET_READ_PERMISSION])
    }

    pub(crate) fn editor_asset_write_capability(&self) -> EditorResult<Capability> {
        self.editor_capability(&[ASSET_WRITE_PERMISSION])
    }

    fn editor_capability(&self, permissions: &[&str]) -> EditorResult<Capability> {
        if self.descriptor_capabilities_required {
            return self
                .capability_template(permissions)
                .ok_or_else(|| Self::missing_capability_error(permissions));
        }

        let capability = Capability::new(self.editor_service.clone(), ServiceRole::Editor);
        let capability = match self.session_id {
            Some(session_id) => capability.with_session(session_id),
            None => capability,
        };
        Ok(capability
            .with_audience(ASSET_PROCESSOR_AUDIENCE)
            .with_permissions(permissions.iter().copied()))
    }

    fn capability_template(&self, permissions: &[&str]) -> Option<Capability> {
        // Asset-processor capabilities are project-scoped and attach each read
        // to the service-owned workspace. Templates are therefore minted
        // unscoped regardless of this client's editor session.
        self.capability_templates
            .iter()
            .find(|capability| {
                capability.matches_brokered_template_request(
                    ServiceRole::Editor,
                    ASSET_PROCESSOR_AUDIENCE,
                    permissions,
                    None,
                )
            })
            .cloned()
    }

    fn missing_capability_error(permissions: &[&str]) -> EditorError {
        EditorError::ServiceDiscovery(format!(
            "asset-processor descriptor did not grant editor capability `{}`",
            permissions.join(", ")
        ))
    }

    #[must_use]
    fn from_descriptor_client(
        client: asset_capnp::asset_processor::Client,
        capability_templates: Vec<Capability>,
    ) -> Self {
        Self {
            client,
            editor_service: ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            session_id: None,
            capability_templates,
            descriptor_capabilities_required: true,
        }
    }
}

impl AssetProcessorEventStreamClient {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub(crate) async fn subscribe_events(
        &self,
        tx: Sender<AssetProcessorEvent>,
    ) -> EditorResult<AssetProcessorEventSubscriptionResult> {
        self.client.subscribe_events(tx).await
    }

    #[must_use]
    pub(crate) fn connection_closed(&self) -> tokio::sync::watch::Receiver<()> {
        self.connection.connection_closed()
    }
}
