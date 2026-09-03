//! `AssetBuilder` adapter for Bink media sources.
//!
//! Bink files are runtime media, not transformed engine data. The
//! builder preserves bytes and places them where `LyShine` expects
//! video assets in the product tree.

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::{engine_path, source_extension_key};
use uuid::uuid;

use crate::{BinkMediaSourceFormat, BinkVideoProductFormat};

pub const NAME: &str = "bink.media";
pub const ID: BuilderId = BuilderId::new(uuid!("b1d7a3e5-c4f8-46ab-9d12-7e3a8c4b5d61"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<BinkMediaSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<BinkVideoProductFormat>()
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

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    ProcessJobResponse {
        products: vec![transform_product(&req.source_path, req.source_bytes)],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

#[must_use]
pub fn transform_product(source_path: &str, source_bytes: &[u8]) -> BuildProduct {
    let ext = source_extension_key(source_path).unwrap_or_else(|| "bk2".to_string());
    TypedBuildProduct::<BinkVideoProductFormat>::from_trusted_path(
        engine_path(source_path, "ui", &ext),
        0,
        source_bytes.to_vec(),
    )
    .erase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::Registries;

    #[test]
    fn descriptor_claims_bink_sources() {
        let registries = Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("ui/videos/splash.bk2"));
        assert!(desc.matches("cinematics/intro.bik"));
        assert!(!desc.matches("cinematics/intro.bin"));
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
            "ui/videos/splash.bk2",
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
