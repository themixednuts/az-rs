use std::path::Path;

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, SourceFileDependency,
    TypedBuildProduct,
};
use az_filesystem::normalize_source_path;
use half::f16;
use intel_tex_2::{RgSurface, RgbaSurface};
use uuid::uuid;

use crate::{
    DecodedFloatImage, DecodedImage, DecodedImage16, DecodedPng, TextureAuthoringFormat,
    TextureColorSpace, TextureCompressionFormat, TextureDimension, TextureImageOrder,
    TextureKtx2ProductFormat, TextureRole, TextureSourceFormat, TextureSourceSettings,
    TextureSourceShape,
    ktx2_writer::{self, Ktx2TextureShape, Ktx2WriteError},
    read_png, read_rgba32f_exr, read_texture_settings, texture_settings_source_path,
};

pub const NAME: &str = "azoth.texture";
pub const ID: BuilderId = BuilderId::new(uuid!("e8ad2898-e988-44e4-bd58-47db1c8fe904"));
pub const VERSION: u32 = 1;

pub const VK_FORMAT_BC5_UNORM_BLOCK: u32 = 141;
pub const VK_FORMAT_BC6H_UFLOAT_BLOCK: u32 = 143;
pub const VK_FORMAT_BC7_UNORM_BLOCK: u32 = 145;
pub const VK_FORMAT_BC7_SRGB_BLOCK: u32 = 146;
pub const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<TextureSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<TextureKtx2ProductFormat>()
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
        source_dependencies: vec![SourceFileDependency::Path(texture_settings_source_path(
            &req.source_path,
        ))],
        ..CreateJobsResponse::default()
    }
}

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product = match transform_product_from_source_root(
        req.source_root,
        &req.source_path,
        req.source_bytes,
    ) {
        Ok(product) => product,
        Err(err) => {
            tracing::warn!(source = %req.source_path, error = %err, "texture product failed");
            return ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            };
        }
    };

    ProcessJobResponse {
        products: vec![product],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

/// # Errors
///
/// Returns [`TextureProductError::Io`] if the sibling `.texture.ron` settings
/// file cannot be read, or any error [`transform_product`] returns.
pub fn transform_product_from_source_root(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, TextureProductError> {
    let settings_path = texture_settings_source_path(source_path);
    let settings_bytes = std::fs::read(source_root.join(&settings_path)).map_err(|source| {
        TextureProductError::Io {
            path: settings_path,
            source,
        }
    })?;
    transform_product(source_path, source_bytes, &settings_bytes)
}

/// # Errors
///
/// Returns [`TextureProductError`] if the settings cannot be parsed, the image
/// cannot be decoded, its dimensions do not match the declared shape, or the
/// encoded product cannot be built.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
    settings_bytes: &[u8],
) -> Result<BuildProduct, TextureProductError> {
    let settings = read_texture_settings(settings_bytes)?;
    let (_vk_format, ktx2) = match source_extension(source_path).as_deref() {
        Some("png") => {
            if settings.authoring_format == TextureAuthoringFormat::Exr {
                return Err(TextureProductError::FormatMismatch {
                    source_path: source_path.to_string(),
                    settings_format: settings.authoring_format,
                });
            }
            if settings.role == TextureRole::Hdr {
                return Err(TextureProductError::UnsupportedPngRole {
                    source_path: source_path.to_string(),
                    role: settings.role,
                });
            }
            let decoded = read_png(source_bytes)?;
            encode_png(decoded, &settings)?
        }
        Some("exr") => {
            if settings.authoring_format != TextureAuthoringFormat::Exr {
                return Err(TextureProductError::FormatMismatch {
                    source_path: source_path.to_string(),
                    settings_format: settings.authoring_format,
                });
            }
            if settings.role != TextureRole::Hdr {
                return Err(TextureProductError::UnsupportedExrRole {
                    source_path: source_path.to_string(),
                    role: settings.role,
                });
            }
            let decoded = read_rgba32f_exr(source_bytes)?;
            encode_hdr_exr(&decoded, &settings)?
        }
        _ => {
            return Err(TextureProductError::UnsupportedExtension {
                source_path: source_path.to_string(),
            });
        }
    };

    Ok(
        TypedBuildProduct::<TextureKtx2ProductFormat>::from_trusted_path(
            texture_product_path(source_path),
            0,
            ktx2,
        )
        .erase(),
    )
}

fn encode_png(
    decoded: DecodedPng,
    settings: &TextureSourceSettings,
) -> Result<(u32, Vec<u8>), TextureProductError> {
    let image = match (decoded, settings.authoring_format) {
        (DecodedPng::Rgba8(image), TextureAuthoringFormat::Png8) => image,
        (DecodedPng::Rgba16(image), TextureAuthoringFormat::Png16) => rgba16_to_rgba8(image),
        (_, TextureAuthoringFormat::Exr) => unreachable!("EXR was rejected before PNG decode"),
        (_, settings_format) => {
            return Err(TextureProductError::PngBitDepthMismatch { settings_format });
        }
    };

    let layout = SourceImageLayout::new(image.width, image.height, settings.shape.as_ref())?;
    match settings.role {
        TextureRole::Normal => {
            let mut blocks = Vec::new();
            for rgba in layout.rgba8_images(&image.rgba)? {
                let (data, width, height) =
                    pad_rgba8_to_rg_blocks(layout.pixel_width, layout.image_height, rgba)?;
                let surface = RgSurface {
                    data: &data,
                    width,
                    height,
                    stride: width
                        .checked_mul(2)
                        .ok_or(TextureProductError::SizeOverflow)?,
                };
                blocks.extend_from_slice(&intel_tex_2::bc5::compress_blocks(&surface));
            }
            Ok((
                VK_FORMAT_BC5_UNORM_BLOCK,
                ktx2_writer::write_single_mip_with_shape(
                    VK_FORMAT_BC5_UNORM_BLOCK,
                    layout.pixel_width,
                    layout.ktx2,
                    &blocks,
                )?,
            ))
        }
        TextureRole::Albedo | TextureRole::Orm | TextureRole::Mask | TextureRole::Ui => {
            let vk_format = if settings.color_space == TextureColorSpace::Srgb {
                VK_FORMAT_BC7_SRGB_BLOCK
            } else {
                VK_FORMAT_BC7_UNORM_BLOCK
            };
            let encode_settings = intel_tex_2::bc7::alpha_basic_settings();
            let mut blocks = Vec::new();
            for rgba in layout.rgba8_images(&image.rgba)? {
                let (data, width, height) =
                    pad_rgba8_to_block_extent(layout.pixel_width, layout.image_height, rgba)?;
                let surface = RgbaSurface {
                    data: &data,
                    width,
                    height,
                    stride: width
                        .checked_mul(4)
                        .ok_or(TextureProductError::SizeOverflow)?,
                };
                blocks.extend_from_slice(&intel_tex_2::bc7::compress_blocks(
                    &encode_settings,
                    &surface,
                ));
            }
            Ok((
                vk_format,
                ktx2_writer::write_single_mip_with_shape(
                    vk_format,
                    layout.pixel_width,
                    layout.ktx2,
                    &blocks,
                )?,
            ))
        }
        TextureRole::Data => {
            if let Some(compression) = &settings.compression
                && compression.format != TextureCompressionFormat::Uncompressed
            {
                return Err(TextureProductError::UnsupportedCompression {
                    role: TextureRole::Data,
                    format: compression.format,
                });
            }
            layout.validate_components(4, image.rgba.len())?;
            Ok((
                VK_FORMAT_R8G8B8A8_UNORM,
                ktx2_writer::write_single_mip_with_shape(
                    VK_FORMAT_R8G8B8A8_UNORM,
                    layout.pixel_width,
                    layout.ktx2,
                    &image.rgba,
                )?,
            ))
        }
        TextureRole::Hdr => Err(TextureProductError::UnsupportedPngRole {
            source_path: "<decoded png>".to_string(),
            role: TextureRole::Hdr,
        }),
    }
}

fn encode_hdr_exr(
    image: &DecodedFloatImage,
    source_settings: &TextureSourceSettings,
) -> Result<(u32, Vec<u8>), TextureProductError> {
    let layout = SourceImageLayout::new(image.width, image.height, source_settings.shape.as_ref())?;
    let settings = intel_tex_2::bc6h::basic_settings();
    let mut blocks = Vec::new();
    for rgba in layout.rgba32f_images(&image.rgba)? {
        let (data, width, height) =
            pad_rgba32f_to_half_blocks(layout.pixel_width, layout.image_height, rgba)?;
        let surface = RgbaSurface {
            data: &data,
            width,
            height,
            stride: width
                .checked_mul(8)
                .ok_or(TextureProductError::SizeOverflow)?,
        };
        blocks.extend_from_slice(&intel_tex_2::bc6h::compress_blocks(&settings, &surface));
    }
    Ok((
        VK_FORMAT_BC6H_UFLOAT_BLOCK,
        ktx2_writer::write_single_mip_with_shape(
            VK_FORMAT_BC6H_UFLOAT_BLOCK,
            layout.pixel_width,
            layout.ktx2,
            &blocks,
        )?,
    ))
}

fn rgba16_to_rgba8(image: DecodedImage16) -> DecodedImage {
    DecodedImage {
        width: image.width,
        height: image.height,
        rgba: image
            .rgba
            .into_iter()
            .map(|sample| (sample / 257) as u8)
            .collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceImageLayout {
    pixel_width: u32,
    image_height: u32,
    image_count: usize,
    ktx2: Ktx2TextureShape,
}

impl SourceImageLayout {
    fn new(
        pixel_width: u32,
        atlas_height: u32,
        shape: Option<&TextureSourceShape>,
    ) -> Result<Self, TextureProductError> {
        if pixel_width == 0 || atlas_height == 0 {
            return Err(TextureProductError::EmptyImage);
        }
        let Some(shape) = shape else {
            return Ok(Self {
                pixel_width,
                image_height: atlas_height,
                image_count: 1,
                ktx2: Ktx2TextureShape {
                    pixel_height: atlas_height,
                    pixel_depth: 0,
                    layer_count: 0,
                    face_count: 1,
                },
            });
        };

        if shape.image_order != TextureImageOrder::LayerFaceDepth {
            return Err(TextureProductError::InvalidShape {
                reason: "unsupported image order",
            });
        }
        if shape.image_height == 0
            || shape.depth == 0
            || shape.array_layers == 0
            || !matches!(shape.faces, 1 | 6)
        {
            return Err(TextureProductError::InvalidShape {
                reason: "image height, depth, layers, and faces must describe non-empty images",
            });
        }
        match shape.dimension {
            TextureDimension::One => {
                if shape.image_height != 1
                    || shape.depth != 1
                    || shape.faces != 1
                    || shape.array_layers == 0
                {
                    return Err(TextureProductError::InvalidShape {
                        reason: "1D textures require height 1, depth 1, and one face",
                    });
                }
            }
            TextureDimension::Two => {
                if shape.depth != 1 {
                    return Err(TextureProductError::InvalidShape {
                        reason: "2D textures require depth 1",
                    });
                }
                if shape.faces == 6 && pixel_width != shape.image_height {
                    return Err(TextureProductError::InvalidShape {
                        reason: "cubemap faces must be square",
                    });
                }
            }
            TextureDimension::Three => {
                if shape.array_layers != 1 || shape.faces != 1 {
                    return Err(TextureProductError::InvalidShape {
                        reason: "3D textures cannot be arrays or cubemaps",
                    });
                }
            }
        }

        let image_count =
            usize::try_from(shape.image_count()).map_err(|_| TextureProductError::SizeOverflow)?;
        let expected_atlas_height = shape
            .atlas_height()
            .ok_or(TextureProductError::SizeOverflow)?;
        if u64::from(atlas_height) != expected_atlas_height {
            return Err(TextureProductError::AtlasShape {
                image_height: shape.image_height,
                image_count: u64::try_from(image_count)
                    .map_err(|_| TextureProductError::SizeOverflow)?,
                atlas_height,
            });
        }

        Ok(Self {
            pixel_width,
            image_height: shape.image_height,
            image_count,
            ktx2: Ktx2TextureShape {
                pixel_height: if shape.dimension == TextureDimension::One {
                    0
                } else {
                    shape.image_height
                },
                pixel_depth: if shape.dimension == TextureDimension::Three {
                    shape.depth
                } else {
                    0
                },
                layer_count: if shape.array_layers > 1 {
                    shape.array_layers
                } else {
                    0
                },
                face_count: shape.faces,
            },
        })
    }

    fn validate_components(
        self,
        components: u32,
        actual: usize,
    ) -> Result<(), TextureProductError> {
        validate_image_len(
            self.pixel_width,
            self.image_height,
            components,
            actual.checked_div(self.image_count).unwrap_or(0),
        )?;
        let expected = checked_len(self.pixel_width, self.image_height, components)?
            .checked_mul(self.image_count)
            .ok_or(TextureProductError::SizeOverflow)?;
        if actual != expected {
            return Err(TextureProductError::ImageSize {
                expected: u64::try_from(expected).map_err(|_| TextureProductError::SizeOverflow)?,
                actual,
            });
        }
        Ok(())
    }

    fn rgba8_images(self, rgba: &[u8]) -> Result<impl Iterator<Item = &[u8]>, TextureProductError> {
        self.validate_components(4, rgba.len())?;
        let image_len = checked_len(self.pixel_width, self.image_height, 4)?;
        Ok(rgba.chunks_exact(image_len))
    }

    fn rgba32f_images(
        self,
        rgba: &[f32],
    ) -> Result<impl Iterator<Item = &[f32]>, TextureProductError> {
        self.validate_components(4, rgba.len())?;
        let image_len = checked_len(self.pixel_width, self.image_height, 4)?;
        Ok(rgba.chunks_exact(image_len))
    }
}

fn pad_rgba8_to_block_extent(
    image_width: u32,
    image_height: u32,
    rgba: &[u8],
) -> Result<(Vec<u8>, u32, u32), TextureProductError> {
    validate_image_len(image_width, image_height, 4, rgba.len())?;
    let width = align_block_extent(image_width)?;
    let height = align_block_extent(image_height)?;
    let len = checked_len(width, height, 4)?;
    let mut data = vec![0; len];
    for y in 0..height {
        let src_y = y.min(image_height - 1);
        for x in 0..width {
            let src_x = x.min(image_width - 1);
            let src = usize::try_from((src_y * image_width + src_x) * 4)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            let dst = usize::try_from((y * width + x) * 4)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            data[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    Ok((data, width, height))
}

fn pad_rgba8_to_rg_blocks(
    image_width: u32,
    image_height: u32,
    rgba: &[u8],
) -> Result<(Vec<u8>, u32, u32), TextureProductError> {
    validate_image_len(image_width, image_height, 4, rgba.len())?;
    let width = align_block_extent(image_width)?;
    let height = align_block_extent(image_height)?;
    let len = checked_len(width, height, 2)?;
    let mut data = vec![0; len];
    for y in 0..height {
        let src_y = y.min(image_height - 1);
        for x in 0..width {
            let src_x = x.min(image_width - 1);
            let src = usize::try_from((src_y * image_width + src_x) * 4)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            let dst = usize::try_from((y * width + x) * 2)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            data[dst] = rgba[src];
            data[dst + 1] = rgba[src + 1];
        }
    }
    Ok((data, width, height))
}

fn pad_rgba32f_to_half_blocks(
    image_width: u32,
    image_height: u32,
    rgba: &[f32],
) -> Result<(Vec<u8>, u32, u32), TextureProductError> {
    validate_image_len(image_width, image_height, 4, rgba.len())?;
    let width = align_block_extent(image_width)?;
    let height = align_block_extent(image_height)?;
    let len = checked_len(width, height, 8)?;
    let mut data = vec![0; len];
    for y in 0..height {
        let src_y = y.min(image_height - 1);
        for x in 0..width {
            let src_x = x.min(image_width - 1);
            let src = usize::try_from((src_y * image_width + src_x) * 4)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            let dst = usize::try_from((y * width + x) * 8)
                .map_err(|_| TextureProductError::SizeOverflow)?;
            for channel in 0..4 {
                let sample = rgba[src + channel].max(0.0);
                let bytes = f16::from_f32(sample).to_bits().to_le_bytes();
                data[dst + channel * 2..dst + channel * 2 + 2].copy_from_slice(&bytes);
            }
        }
    }
    Ok((data, width, height))
}

fn align_block_extent(value: u32) -> Result<u32, TextureProductError> {
    if value == 0 {
        return Err(TextureProductError::EmptyImage);
    }
    value
        .checked_add(3)
        .map(|value| value & !3)
        .ok_or(TextureProductError::SizeOverflow)
}

fn validate_image_len(
    width: u32,
    height: u32,
    components: u32,
    actual: usize,
) -> Result<(), TextureProductError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(u64::from(components)))
        .ok_or(TextureProductError::SizeOverflow)?;
    if u64::try_from(actual).map_err(|_| TextureProductError::SizeOverflow)? != expected {
        return Err(TextureProductError::ImageSize { expected, actual });
    }
    Ok(())
}

fn checked_len(
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
) -> Result<usize, TextureProductError> {
    usize::try_from(
        u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(u64::from(bytes_per_pixel)))
            .ok_or(TextureProductError::SizeOverflow)?,
    )
    .map_err(|_| TextureProductError::SizeOverflow)
}

#[must_use]
pub fn texture_product_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".png")
        .or_else(|| normalized.strip_suffix(".exr"))
        .unwrap_or(&normalized);
    if stem.starts_with("textures/") {
        format!("{stem}.ktx2")
    } else {
        format!("textures/{stem}.ktx2")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TextureProductError {
    #[error("read texture settings {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("parse texture settings: {0}")]
    Settings(#[from] crate::TextureSettingsError),
    #[error("read PNG texture source: {0}")]
    Png(#[from] crate::PngCodecError),
    #[error("read EXR texture source: {0}")]
    Exr(#[from] crate::ExrCodecError),
    #[error("write KTX2 texture product: {0}")]
    Ktx2(#[from] Ktx2WriteError),
    #[error("texture settings format {settings_format:?} does not match source {source_path}")]
    FormatMismatch {
        source_path: String,
        settings_format: TextureAuthoringFormat,
    },
    #[error("PNG bit depth does not match texture settings format {settings_format:?}")]
    PngBitDepthMismatch {
        settings_format: TextureAuthoringFormat,
    },
    #[error("PNG source {source_path} cannot be encoded for role {role:?}")]
    UnsupportedPngRole {
        source_path: String,
        role: TextureRole,
    },
    #[error("texture role {role:?} does not support compression format {format:?}")]
    UnsupportedCompression {
        role: TextureRole,
        format: TextureCompressionFormat,
    },
    #[error("EXR source {source_path} cannot be encoded for role {role:?}")]
    UnsupportedExrRole {
        source_path: String,
        role: TextureRole,
    },
    #[error("unsupported texture source extension for {source_path}")]
    UnsupportedExtension { source_path: String },
    #[error("image dimensions must be non-zero")]
    EmptyImage,
    #[error("image contains {actual} components, expected {expected}")]
    ImageSize { expected: u64, actual: usize },
    #[error("invalid texture source shape: {reason}")]
    InvalidShape { reason: &'static str },
    #[error(
        "texture source atlas height {atlas_height} does not contain {image_count} images of height {image_height}"
    )]
    AtlasShape {
        image_height: u32,
        image_count: u64,
        atlas_height: u32,
    },
    #[error("image dimensions are too large")]
    SizeOverflow,
}

fn source_extension(source_path: &str) -> Option<String> {
    normalize_source_path(source_path)
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use az_gem_contract::Registries;
    use exr::prelude::{Image, SpecificChannels, Vec2, WritableImage};

    use super::*;

    #[test]
    fn descriptor_claims_texture_png_and_exr_sources() {
        let registries = Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("textures/characters/player/body_d.png"));
        assert!(desc.matches("textures/characters/player/body_d.exr"));
        assert!(!desc.matches("textures/characters/player/body_d.texture.ron"));
        assert!(!desc.matches("textures/characters/player/body_d.dds"));
        assert!(!desc.matches("textures/characters/player/body_d.ktx2"));
    }

    #[test]
    fn create_jobs_depends_on_texture_settings_sidecar() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        let desc = desc(&context);
        let response = (desc.create_jobs)(&CreateJobsRequest::new(
            ID,
            "textures/test/checker.png",
            Path::new(""),
            uuid::Uuid::nil(),
            Some(crate::source_schemas::TEXTURE.as_str()),
            &[],
            &[az_asset_builder::DEFAULT_PLATFORM_ID],
            &context,
        ));

        assert_eq!(
            response.source_dependencies,
            vec![SourceFileDependency::Path(
                "textures/test/checker.texture.ron".to_string()
            )]
        );
    }

    #[test]
    fn product_path_keeps_texture_root() {
        assert_eq!(
            texture_product_path("textures/characters/player/body_d.png"),
            "textures/characters/player/body_d.ktx2"
        );
        assert_eq!(
            texture_product_path("characters/player/body_d.exr"),
            "textures/characters/player/body_d.ktx2"
        );
    }

    #[test]
    fn transform_png_albedo_product_uses_bc7_srgb_from_settings() {
        let png = rgba8_png(4, 4, [255, 0, 0, 255]);
        let settings = settings(
            TextureAuthoringFormat::Png8,
            TextureColorSpace::Srgb,
            TextureRole::Albedo,
        );

        let product = transform_product("textures/test/checker.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();
        let levels = reader.levels().collect::<Vec<_>>();

        assert_eq!(product.product_path, "textures/test/checker.ktx2");
        assert_eq!(product.format, crate::product_formats::KTX2);
        assert_eq!(reader.header().pixel_width, 4);
        assert_eq!(reader.header().pixel_height, 4);
        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC7_SRGB_BLOCK)
        );
        assert_eq!(reader.header().level_count, 1);
        assert_eq!(levels[0].data.len(), 16);
    }

    #[test]
    fn transform_png_normal_product_uses_bc5_from_settings() {
        let png = rgba8_png(4, 4, [128, 128, 255, 255]);
        let settings = settings(
            TextureAuthoringFormat::Png8,
            TextureColorSpace::Linear,
            TextureRole::Normal,
        );

        let product = transform_product("textures/test/normal.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC5_UNORM_BLOCK)
        );
    }

    #[test]
    fn transform_png_orm_product_uses_bc7_unorm_from_settings() {
        let png = rgba8_png(4, 4, [128, 64, 32, 255]);
        let settings = settings(
            TextureAuthoringFormat::Png8,
            TextureColorSpace::Linear,
            TextureRole::Orm,
        );

        let product = transform_product("textures/test/packed_orm.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC7_UNORM_BLOCK)
        );
    }

    #[test]
    fn transform_png_mask_product_uses_bc7_unorm_and_keeps_all_channels() {
        let png = rgba8_png(4, 4, [128, 64, 32, 16]);
        let settings = settings(
            TextureAuthoringFormat::Png8,
            TextureColorSpace::Linear,
            TextureRole::Mask,
        );

        let product = transform_product("textures/test/opacity.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();
        let levels = reader.levels().collect::<Vec<_>>();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC7_UNORM_BLOCK)
        );
        assert_eq!(levels[0].data.len(), 16);
    }

    #[test]
    fn transform_png_data_product_uses_uncompressed_rgba8_from_settings() {
        let png = rgba8_png(2, 2, [7, 7, 7, 255]);
        let settings = crate::write_texture_settings(&crate::TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Linear,
            role: TextureRole::Data,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: Some(crate::TextureCompressionIntent {
                format: TextureCompressionFormat::Uncompressed,
            }),
            shape: None,
        })
        .unwrap();

        let product = transform_product("textures/test/mask.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();
        let levels = reader.levels().collect::<Vec<_>>();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_R8G8B8A8_UNORM)
        );
        assert_eq!(levels[0].data.len(), 16);
        assert_eq!(levels[0].data, [7, 7, 7, 255].repeat(4));
    }

    #[test]
    fn transform_png_cubemap_array_restores_ktx2_shape_and_image_order() {
        let mut rgba = Vec::new();
        for image in 0..12u8 {
            rgba.extend_from_slice(&[image, 0, 0, 255].repeat(4));
        }
        let png = rgba8_png_data(2, 24, rgba);
        let settings = crate::write_texture_settings(&crate::TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Linear,
            role: TextureRole::Data,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: Some(crate::TextureCompressionIntent {
                format: TextureCompressionFormat::Uncompressed,
            }),
            shape: Some(TextureSourceShape {
                dimension: TextureDimension::Two,
                image_height: 2,
                depth: 1,
                array_layers: 2,
                faces: 6,
                image_order: TextureImageOrder::LayerFaceDepth,
            }),
        })
        .unwrap();

        let product = transform_product("textures/test/cubes.png", &png, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();
        let level = reader.levels().next().unwrap();

        assert_eq!(reader.header().pixel_width, 2);
        assert_eq!(reader.header().pixel_height, 2);
        assert_eq!(reader.header().pixel_depth, 0);
        assert_eq!(reader.header().layer_count, 2);
        assert_eq!(reader.header().face_count, 6);
        assert_eq!(level.data.len(), 2 * 2 * 4 * 12);
        for (image, pixels) in level.data.chunks_exact(2 * 2 * 4).enumerate() {
            let expected = u8::try_from(image).unwrap();
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[0] == expected));
        }
    }

    #[test]
    fn transform_exr_hdr_product_uses_bc6h_from_settings() {
        let exr = rgba32f_exr(4, 4, &[1.0, 0.25, 0.0, 1.0]);
        let settings = settings(
            TextureAuthoringFormat::Exr,
            TextureColorSpace::Linear,
            TextureRole::Hdr,
        );

        let product = transform_product("textures/test/skyprobe.exr", &exr, &settings).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();
        let levels = reader.levels().collect::<Vec<_>>();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC6H_UFLOAT_BLOCK)
        );
        assert_eq!(levels[0].data.len(), 16);
    }

    #[test]
    fn transform_from_source_root_reads_settings_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = "textures/test/checker.png";
        let settings_path = temp.path().join("textures/test/checker.texture.ron");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            &settings_path,
            settings(
                TextureAuthoringFormat::Png8,
                TextureColorSpace::Srgb,
                TextureRole::Albedo,
            ),
        )
        .unwrap();
        let png = rgba8_png(4, 4, [255, 255, 255, 255]);

        let product = transform_product_from_source_root(temp.path(), source_path, &png).unwrap();
        let reader = ktx2::Reader::new(&product.bytes).unwrap();

        assert_eq!(
            reader.header().format.map(|format| format.value()),
            Some(VK_FORMAT_BC7_SRGB_BLOCK)
        );
    }

    fn rgba8_png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        rgba8_png_data(width, height, rgba)
    }

    fn rgba8_png_data(width: u32, height: u32, rgba: Vec<u8>) -> Vec<u8> {
        let image = image::RgbaImage::from_vec(width, height, rgba).unwrap();
        let mut cursor = Cursor::new(Vec::new());
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    fn rgba32f_exr(width: u32, height: u32, pixel: &[f32; 4]) -> Vec<u8> {
        let width_usize = usize::try_from(width).unwrap();
        let height_usize = usize::try_from(height).unwrap();
        let channels = SpecificChannels::rgba(|_position: Vec2<usize>| {
            (pixel[0], pixel[1], pixel[2], pixel[3])
        });
        let image = Image::from_channels((width_usize, height_usize), channels);
        let mut cursor = Cursor::new(Vec::new());
        image
            .write()
            .non_parallel()
            .to_buffered(&mut cursor)
            .unwrap();
        cursor.into_inner()
    }

    fn settings(
        authoring_format: TextureAuthoringFormat,
        color_space: TextureColorSpace,
        role: TextureRole,
    ) -> Vec<u8> {
        crate::write_texture_settings(&crate::TextureSourceSettings {
            authoring_format,
            color_space,
            role,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: None,
            shape: None,
        })
        .unwrap()
    }
}
