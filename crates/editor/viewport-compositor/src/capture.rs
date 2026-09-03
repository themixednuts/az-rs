//! Windows Graphics Capture recording and per-frame compositor classification.
//!
//! WGC composes the window the way the desktop compositor does, so this sees
//! both the Bevy and GPUI `DirectComposition` sibling visuals. There is no
//! counterpart on other platforms, which is why the whole module is Windows
//! only.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba};
use serde::Serialize;
use windows_capture::{
    capture::{Context as CaptureContext, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

use crate::{
    guard_existing,
    pixel::{LAYOUT_CORNER, Rect, bgra_pixel, color_bounds_bgra, color_near},
};

const RENDER_BORDER: [u8; 4] = [0xff, 0x00, 0xff, 0xff];
const BIT_ON: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
const BIT_OFF: [u8; 4] = [0x00, 0xff, 0xff, 0xff];

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Classification {
    Good,
    ScaledComplete,
    Stretched,
    Overcopied,
    Uninitialized,
}

/// Binary frame id and render extents the producer paints inside the layout
/// rect. Bilinear filtering during a resize can leave any of them undecodable,
/// which is why each field is independently optional.
#[derive(Clone, Copy, Debug, Default)]
struct DiagnosticMetadata {
    frame_id: Option<u32>,
    encoded_width: Option<u32>,
    encoded_height: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
struct FrameResult {
    index: u32,
    display_time: u64,
    classification: Classification,
    frame_id: Option<u32>,
    encoded_width: Option<u32>,
    encoded_height: Option<u32>,
    layout_rect: Option<Rect>,
    render_border: Option<Rect>,
}

#[derive(Debug, Serialize)]
struct CaptureReport {
    capture_api: &'static str,
    target_title: String,
    requested_fps: u32,
    total_frames: u32,
    artifact_frames: u32,
    classifications: BTreeMap<Classification, u32>,
    frames: Vec<FrameResult>,
}

pub fn run(
    title: &str,
    fps: u32,
    frame_count: u32,
    output: &Path,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let mut matches = Window::enumerate()?
        .into_iter()
        .filter(|window| {
            window
                .title()
                .is_ok_and(|candidate| candidate.contains(title))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        let labels = matches
            .iter()
            .filter_map(|window| window.title().ok())
            .collect::<Vec<_>>();
        bail!(
            "expected one WGC window containing {title:?}, found {}: {labels:?}",
            matches.len()
        );
    }
    let target = matches.pop().expect("one target checked above");
    let target_title = target.title()?;
    for artifact in ["capture.json", "first.png", "first-artifact.png"] {
        guard_existing(&output.join(artifact), force)?;
    }
    if dry_run {
        println!(
            "dry-run: capture {frame_count} frame(s) from `{target_title}` into {}",
            output.display()
        );
        return Ok(());
    }
    let flags = CaptureFlags {
        output: output.to_path_buf(),
        target_title,
        requested_fps: fps,
        requested_frames: frame_count,
    };
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    let settings = Settings::new(
        target,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Custom(interval),
        DirtyRegionSettings::ReportAndRender,
        ColorFormat::Bgra8,
        flags,
    );
    WgcCapture::start(settings).map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[derive(Clone)]
struct CaptureFlags {
    output: PathBuf,
    target_title: String,
    requested_fps: u32,
    requested_frames: u32,
}

struct WgcCapture {
    flags: CaptureFlags,
    frames: Vec<FrameResult>,
    classifications: BTreeMap<Classification, u32>,
    previous_frame_id: Option<u32>,
    saved_first: bool,
    saved_artifact: bool,
}

impl WgcCapture {
    fn finish(&self) -> Result<()> {
        let total_frames = u32::try_from(self.frames.len())
            .context("captured more frames than the report can count")?;
        let good = self
            .classifications
            .get(&Classification::Good)
            .copied()
            .unwrap_or(0);
        let scaled_complete = self
            .classifications
            .get(&Classification::ScaledComplete)
            .copied()
            .unwrap_or(0);
        let report = CaptureReport {
            capture_api: "Windows.Graphics.Capture",
            target_title: self.flags.target_title.clone(),
            requested_fps: self.flags.requested_fps,
            total_frames,
            artifact_frames: total_frames.saturating_sub(good + scaled_complete),
            classifications: self.classifications.clone(),
            frames: self.frames.clone(),
        };
        fs::write(
            self.flags.output.join("capture.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!(
            "artifact frames: {}/{}",
            report.artifact_frames, report.total_frames
        );
        for (class, count) in &report.classifications {
            println!("{class:?}: {count}");
        }
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for WgcCapture {
    type Flags = CaptureFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(context: CaptureContext<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        fs::create_dir_all(&context.flags.output)?;
        Ok(Self {
            frames: Vec::with_capacity(context.flags.requested_frames as usize),
            classifications: BTreeMap::new(),
            previous_frame_id: None,
            saved_first: false,
            saved_artifact: false,
            flags: context.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();
        // `max` already clamps the 100ns timestamp, so this never truncates.
        let display_time = u64::try_from(frame.timestamp().Duration.max(0))
            .context("WGC frame timestamp did not fit an unsigned duration")?;
        let mut buffer = frame.buffer()?;
        let bgra = buffer.as_nopadding_buffer()?;
        let index = u32::try_from(self.frames.len())
            .context("captured more frames than the report can index")?;
        let result = analyze_frame(
            index,
            display_time,
            width,
            height,
            bgra,
            self.previous_frame_id,
        );
        if result.frame_id.is_some() {
            self.previous_frame_id = result.frame_id;
        }
        *self
            .classifications
            .entry(result.classification)
            .or_insert(0) += 1;
        if !self.saved_first {
            save_bgra_png(&self.flags.output.join("first.png"), width, height, bgra)?;
            self.saved_first = true;
        }
        if !matches!(
            result.classification,
            Classification::Good | Classification::ScaledComplete
        ) && !self.saved_artifact
        {
            save_bgra_png(
                &self.flags.output.join("first-artifact.png"),
                width,
                height,
                bgra,
            )?;
            self.saved_artifact = true;
        }
        self.frames.push(result);
        if self.frames.len() >= self.flags.requested_frames as usize {
            self.finish()?;
            capture_control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        self.finish()?;
        Ok(())
    }
}

fn analyze_frame(
    index: u32,
    display_time: u64,
    width: u32,
    height: u32,
    bgra: &[u8],
    previous_frame_id: Option<u32>,
) -> FrameResult {
    let layout_rect = color_bounds_bgra(bgra, width, height, LAYOUT_CORNER, 18);
    let render_border = color_bounds_bgra(bgra, width, height, RENDER_BORDER, 18);
    // Without the lime corner markers there is no layout rect to decode the
    // metadata relative to, so the frame carries no diagnostic at all.
    let (classification, metadata) = layout_rect.map_or_else(
        || (Classification::Uninitialized, DiagnosticMetadata::default()),
        |layout| {
            let metadata = decode_metadata(bgra, width, height, layout);
            let classification =
                classify_diagnostic(layout, render_border, metadata, previous_frame_id);
            (classification, metadata)
        },
    );
    FrameResult {
        index,
        display_time,
        classification,
        frame_id: metadata.frame_id,
        encoded_width: metadata.encoded_width,
        encoded_height: metadata.encoded_height,
        layout_rect,
        render_border,
    }
}

fn decode_metadata(bgra: &[u8], width: u32, height: u32, layout: Rect) -> DiagnosticMetadata {
    let metadata_x = layout.left + layout.width().saturating_sub(256) / 2;
    DiagnosticMetadata {
        frame_id: decode_bits_bgra(bgra, width, height, metadata_x, layout.top + 20, 32),
        encoded_width: decode_bits_bgra(bgra, width, height, metadata_x, layout.top + 30, 16),
        encoded_height: decode_bits_bgra(bgra, width, height, metadata_x, layout.top + 40, 16),
    }
}

fn classify_diagnostic(
    layout: Rect,
    render_border: Option<Rect>,
    metadata: DiagnosticMetadata,
    _previous_frame_id: Option<u32>,
) -> Classification {
    let DiagnosticMetadata {
        frame_id,
        encoded_width,
        encoded_height,
    } = metadata;
    // During the bounded resize interval DComp scales the last complete
    // producer surface. Its magenta border is transformed with the texture and
    // must still land exactly on the independently painted lime layout rect.
    // Bilinear filtering can make the binary metadata undecodable, so border
    // agreement is the authoritative completeness signal for that interval.
    if render_border == Some(layout) {
        return match (frame_id, encoded_width, encoded_height) {
            (Some(_), Some(render_width), Some(render_height))
                if render_width == layout.width() && render_height == layout.height() =>
            {
                // A repeated complete surface at an unchanged layout is not a
                // compositor artifact; Phase 4 intentionally retains the last
                // complete surface until a replacement is ready. Frame cadence
                // is measured separately from visual integrity.
                Classification::Good
            }
            _ => Classification::ScaledComplete,
        };
    }
    match (frame_id, encoded_width, encoded_height, render_border) {
        (Some(_), Some(render_width), Some(render_height), Some(border))
            if render_width > layout.width()
                || render_height > layout.height()
                || border.width() > layout.width()
                || border.height() > layout.height() =>
        {
            Classification::Overcopied
        }
        (Some(_), Some(_), Some(_), Some(_)) => Classification::Stretched,
        _ => Classification::Uninitialized,
    }
}

fn decode_bits_bgra(
    data: &[u8],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
    bits: u32,
) -> Option<u32> {
    let mut value = 0_u32;
    for bit in 0..bits {
        let x = origin_x + bit * 4 + 1;
        let y = origin_y + 2;
        if x >= width || y >= height {
            return None;
        }
        let pixel = bgra_pixel(data, width, x, y)?;
        let rgba = [pixel[2], pixel[1], pixel[0], pixel[3]];
        if color_near(rgba, BIT_ON, 28) {
            value |= 1 << bit;
        } else if !color_near(rgba, BIT_OFF, 28) {
            return None;
        }
    }
    Some(value)
}

fn save_bgra_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<()> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for pixel in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba)
        .context("WGC frame length did not match its dimensions")?;
    image
        .save(path)
        .with_context(|| format!("saving {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_transformed_border_is_complete_even_when_metadata_is_filtered() {
        let layout = Rect {
            left: 20,
            top: 30,
            right: 1_120,
            bottom: 830,
        };
        assert_eq!(
            classify_diagnostic(
                layout,
                Some(layout),
                DiagnosticMetadata::default(),
                Some(41)
            ),
            Classification::ScaledComplete
        );
    }

    #[test]
    fn exact_size_repeated_complete_frame_remains_good() {
        let layout = Rect {
            left: 20,
            top: 30,
            right: 1_120,
            bottom: 830,
        };
        assert_eq!(
            classify_diagnostic(
                layout,
                Some(layout),
                DiagnosticMetadata {
                    frame_id: Some(42),
                    encoded_width: Some(1_100),
                    encoded_height: Some(800),
                },
                Some(42),
            ),
            Classification::Good
        );
    }

    #[test]
    fn border_geometry_mismatch_is_still_an_artifact() {
        let layout = Rect {
            left: 20,
            top: 30,
            right: 1_120,
            bottom: 830,
        };
        let overcopied = Rect {
            right: 1_140,
            ..layout
        };
        assert_eq!(
            classify_diagnostic(
                layout,
                Some(overcopied),
                DiagnosticMetadata {
                    frame_id: Some(42),
                    encoded_width: Some(1_120),
                    encoded_height: Some(800),
                },
                Some(41),
            ),
            Classification::Overcopied
        );
    }
}
