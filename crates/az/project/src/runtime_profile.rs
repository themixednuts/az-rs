use crate::{
    LockedBuildTarget, LockedServiceTarget, ProjectAgsRuntimeProfile, ProjectLock,
    ProjectManifestError, ProjectNativeRegistrationProfile, ProjectRuntimeProfile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedTargetRuntimeProfile {
    pub project_id: String,
    pub project_name: String,
    pub target_name: String,
    pub settings_name: String,
    pub runtime: ProjectRuntimeProfile,
}

impl LockedTargetRuntimeProfile {
    /// The `[ags]` settings this profile declares.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectManifestError::InvalidProjectLock`] if the resolved
    /// runtime profile carries no `[ags]` table.
    pub fn require_ags(&self) -> Result<ProjectAgsRuntimeProfile, ProjectManifestError> {
        self.runtime.ags.clone().ok_or_else(|| {
            invalid_lock(format!(
                "runtime profile `{}` for target `{}` must declare [ags] settings",
                self.settings_name, self.target_name
            ))
        })
    }

    /// The `[native_registration]` settings this profile declares.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectManifestError::InvalidProjectLock`] if the resolved
    /// runtime profile carries no `[native_registration]` table.
    pub fn require_native_registration(
        &self,
    ) -> Result<ProjectNativeRegistrationProfile, ProjectManifestError> {
        self.runtime.native_registration.ok_or_else(|| {
            invalid_lock(format!(
                "runtime profile `{}` for target `{}` must declare [native_registration] settings",
                self.settings_name, self.target_name
            ))
        })
    }
}

/// Resolve the runtime profile a locked build target points at.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidProjectLock`] if `target_name` names
/// no entry in `lock.tools.build_targets`, if that entry declares no
/// `settings`, or if the `settings` name matches no profile in `lock.runtime`.
pub fn resolve_build_target_runtime_profile(
    lock: &ProjectLock,
    target_name: &str,
) -> Result<LockedTargetRuntimeProfile, ProjectManifestError> {
    let target = lock
        .tools
        .build_targets
        .iter()
        .find(|target| target.name == target_name)
        .ok_or_else(|| invalid_lock(format!("build target `{target_name}` is not locked")))?;
    resolve_target_runtime_profile(
        lock,
        &target.name,
        target.settings.as_deref(),
        "build target",
    )
}

/// Resolve the runtime profile a locked service target points at.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidProjectLock`] if `target_name` names
/// no entry in `lock.tools.service_targets`, if that entry declares no
/// `settings`, or if the `settings` name matches no profile in `lock.runtime`.
pub fn resolve_service_target_runtime_profile(
    lock: &ProjectLock,
    target_name: &str,
) -> Result<LockedTargetRuntimeProfile, ProjectManifestError> {
    let target = lock
        .tools
        .service_targets
        .iter()
        .find(|target| target.name == target_name)
        .ok_or_else(|| invalid_lock(format!("service target `{target_name}` is not locked")))?;
    resolve_target_runtime_profile(
        lock,
        &target.name,
        target.settings.as_deref(),
        "service target",
    )
}

/// Resolve a named project runtime profile for an engine-generated target.
///
/// Generated topology targets are not entries in `tools.build_targets`; their
/// launcher identity is supplied by target generation while their build/auth
/// settings come directly from the project's named runtime profile.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidProjectLock`] if `settings_name`
/// matches no profile in `lock.runtime`.
pub fn resolve_generated_target_runtime_profile(
    lock: &ProjectLock,
    target_name: &str,
    settings_name: &str,
) -> Result<LockedTargetRuntimeProfile, ProjectManifestError> {
    resolve_target_runtime_profile(lock, target_name, Some(settings_name), "generated target")
}

/// The runtime profile name a locked build target declares in `settings`.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidProjectLock`] if `target` declares no
/// `settings`.
pub fn build_target_runtime_settings(
    target: &LockedBuildTarget,
) -> Result<&str, ProjectManifestError> {
    target
        .settings
        .as_deref()
        .ok_or_else(|| target_missing_settings("build target", &target.name))
}

/// The runtime profile name a locked service target declares in `settings`.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidProjectLock`] if `target` declares no
/// `settings`.
pub fn service_target_runtime_settings(
    target: &LockedServiceTarget,
) -> Result<&str, ProjectManifestError> {
    target
        .settings
        .as_deref()
        .ok_or_else(|| target_missing_settings("service target", &target.name))
}

fn resolve_target_runtime_profile(
    lock: &ProjectLock,
    target_name: &str,
    settings: Option<&str>,
    target_kind: &str,
) -> Result<LockedTargetRuntimeProfile, ProjectManifestError> {
    let settings_name =
        settings.ok_or_else(|| target_missing_settings(target_kind, target_name))?;
    let runtime = lock.runtime.profile(settings_name).ok_or_else(|| {
        invalid_lock(format!(
            "{target_kind} `{target_name}` references missing runtime profile `{settings_name}`"
        ))
    })?;
    Ok(LockedTargetRuntimeProfile {
        project_id: lock.project.id.clone(),
        project_name: lock.project.name.clone(),
        target_name: target_name.to_string(),
        settings_name: settings_name.to_string(),
        runtime: runtime.clone(),
    })
}

fn target_missing_settings(target_kind: &str, target_name: &str) -> ProjectManifestError {
    invalid_lock(format!(
        "{target_kind} `{target_name}` must declare a runtime profile in `settings`"
    ))
}

fn invalid_lock(message: impl Into<String>) -> ProjectManifestError {
    ProjectManifestError::InvalidProjectLock {
        message: message.into(),
    }
}
