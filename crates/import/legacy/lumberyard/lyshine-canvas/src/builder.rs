//! `AssetBuilder` adapter for `LyShine` canvas `ObjectStream` sources.
//!
//! Transforming a canvas needs captured `ObjectStream` ClassData/serializer
//! metadata and a selected texture format. Both arrive as arguments to
//! [`transform_product`]; neither is reachable from a host's composition, so
//! this crate contributes its asset type, product format and source schema but
//! not the rule below — see [`crate::build_rules`]. Font-path resolution stays
//! local and identity-mapped until the asset processor owns a richer source
//! index.

use az_asset::EngineTextureFormat;
use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::engine_path;
use az_gem_lyshine::write_ui_canvas_asset;
use az_objectstream::context::ObjectStreamReadContext;
use uuid::uuid;

use crate::{UiCanvasProductFormat, UiCanvasSourceFormat, transform_ui_canvas_asset};

pub const NAME: &str = "lyshine.ui-canvas";
pub const ID: BuilderId = BuilderId::new(uuid!("c2d6a7b8-94b4-4500-1871-e6f3b495a211"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<UiCanvasSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<UiCanvasProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(req: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let jobs = req
        .platforms
        .iter()
        .copied()
        .map(JobDescriptor::default_for_platform)
        .collect();
    CreateJobsResponse {
        jobs,
        ..CreateJobsResponse::default()
    }
}

/// Fails every job, by construction.
///
/// A canvas is read against captured `ObjectStream` `ClassData`, and a host's
/// composition carries no such capture — there is no registry family for one.
/// Rather than emit a canvas decoded against an empty class table, the rule
/// refuses the job and names what is missing. This is why [`crate::build_rules`]
/// is empty: an unregistered rule leaves `.uicanvas` sources unclaimed instead
/// of claiming and failing them.
fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    tracing::warn!(
        source = %req.source_path,
        "ui canvas jobs need captured ObjectStream class data, which no composed \
         registry carries; transform through `transform_product` instead"
    );
    ProcessJobResponse {
        result: ProcessJobResult::Failed,
        ..ProcessJobResponse::default()
    }
}

/// Compiles a `LyShine` canvas `ObjectStream` into its packed build product.
///
/// # Errors
///
/// Returns [`UiCanvasProductError::Transform`] wrapping any error
/// [`transform_ui_canvas_asset`] returns — an unparseable `ObjectStream`, a
/// root that is not a canvas, a missing element or field, or a field whose
/// reflected type the reader does not map. Returns
/// [`UiCanvasProductError::Serialize`] if the assembled canvas cannot be
/// written to its binary product format.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
    context: &ObjectStreamReadContext,
    texture_format: EngineTextureFormat,
) -> Result<BuildProduct, UiCanvasProductError> {
    let asset = transform_ui_canvas_asset(source_bytes, context, texture_format, |source_path| {
        Ok(source_path.to_string())
    })?;
    let mut bytes = Vec::new();
    write_ui_canvas_asset(&asset, &mut bytes)?;

    Ok(
        TypedBuildProduct::<UiCanvasProductFormat>::from_trusted_path(
            engine_path(source_path, "ui", "ui.canvas.bin"),
            0,
            bytes,
        )
        .erase(),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum UiCanvasProductError {
    #[error("transform UI canvas asset: {0}")]
    Transform(#[from] crate::UiCanvasTransformError),
    #[error("serialize UI canvas product: {0}")]
    Serialize(#[from] az_gem_lyshine::UiCanvasAssetFormatError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::Registries;

    #[test]
    fn descriptor_claims_canvas_sources() {
        let registries = Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("lyshineui/main menu/landing.uicanvas"));
        assert!(desc.matches("lyshineui/main menu/landing.dynamicuicanvas"));
        assert!(!desc.matches("lyshineui/main menu/landing.slice"));
    }

    /// The rule is deliberately withheld from the crate's contribution: a host
    /// that claimed `.uicanvas` sources here would fail every one of them.
    #[test]
    fn the_crate_contributes_no_ui_canvas_rule() {
        assert!(crate::build_rules().is_empty());
    }

    #[test]
    fn create_jobs_emits_default_job_for_each_platform() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        let platforms = [
            az_asset_builder::DEFAULT_PLATFORM_ID,
            az_asset_builder::PlatformId::new_static("server"),
        ];
        let req = CreateJobsRequest::new(
            ID,
            "lyshineui/main menu/landing.uicanvas",
            std::path::Path::new(""),
            uuid::Uuid::nil(),
            None,
            b"",
            &platforms,
            &context,
        );

        let response = create_jobs(&req);

        assert_eq!(response.jobs.len(), 2);
        assert_eq!(response.jobs[0].platform, "pc");
        assert_eq!(response.jobs[1].platform, "server");
        assert_eq!(response.jobs[0].job_key, "default");
        assert_eq!(response.jobs[1].job_key, "default");
    }
}
