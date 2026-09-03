//! Sequencer mode center workbench.
//!
//! Editor-core publishes [`EditorSequenceTimeline`] from the selected
//! animation motion's event tracks and owns all transport state. This panel
//! renders that projection and dispatches typed Sequencer actions; it does not
//! load project data or mutate keys. Authored `.seq` documents are a deliberate
//! follow-up seam once project-host exposes a sequence document schema.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Bounds, Context, Div, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Render, Stateful,
    Styled, Window, div, px, relative,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::scroll::ScrollableElement;
use gpui_component::theme::Theme;
use gpui_component::{ActiveTheme, ElementExt, h_flex, v_flex};

use crate::actions::{SeqGoToKey, SeqKeyTarget, SeqPlay, SeqScrub, SeqSetLoop, SeqStop};
use crate::panels::kit;
use crate::panels::render_project_host_connection_placeholder;
use crate::panels::viewport::EditorAnimationMotionData;

pub const SEQUENCER_FPS: u32 = 30;
const RULER_DIVISIONS: u32 = 20;
const TRACK_NAME_WIDTH: f32 = 220.0;
const TRACK_ROW_HEIGHT: f32 = 34.0;

/// Root Sequencer projection published by editor-core.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSequenceTimeline {
    /// The selected motion backing this v1 projection. `None` is the explicit
    /// empty state until a motion is selected and present in the catalog.
    pub source_motion_path: Option<String>,
    pub title: String,
    pub duration_millis: u32,
    pub position_millis: u32,
    pub fps: u32,
    pub playing: bool,
    pub looping: bool,
    pub ruler_ticks: Vec<SequenceRulerTick>,
    pub tracks: Vec<SequenceTrackProjection>,
}

impl EditorSequenceTimeline {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            source_motion_path: None,
            title: "No motion selected".to_owned(),
            duration_millis: 0,
            position_millis: 0,
            fps: SEQUENCER_FPS,
            playing: false,
            looping: true,
            ruler_ticks: Vec::new(),
            tracks: Vec::new(),
        }
    }

    /// Project one selected motion into Sequencer rows. Playback fields are
    /// supplied by editor-core so rebuilding source data preserves transport.
    #[must_use]
    pub fn from_motion(
        motion: &EditorAnimationMotionData,
        position_millis: u32,
        playing: bool,
        looping: bool,
    ) -> Self {
        let duration_millis = motion
            .duration_millis
            .or_else(|| {
                motion
                    .events
                    .iter()
                    .map(|event| event.end_time_millis)
                    .max()
            })
            .unwrap_or(1)
            .max(1);
        Self {
            source_motion_path: Some(motion.asset_path.clone()),
            title: motion.name.clone(),
            duration_millis,
            position_millis: position_millis.min(duration_millis),
            fps: SEQUENCER_FPS,
            playing,
            looping,
            ruler_ticks: sequence_ruler_ticks(duration_millis),
            tracks: sequence_tracks(motion, duration_millis),
        }
    }

    #[must_use]
    pub const fn has_source(&self) -> bool {
        self.source_motion_path.is_some() && self.duration_millis > 0
    }

    #[must_use]
    pub fn key_positions(&self) -> Vec<u32> {
        let mut positions = self
            .tracks
            .iter()
            .flat_map(|track| track.keys.iter().map(|key| key.position_millis))
            .collect::<Vec<_>>();
        positions.sort_unstable();
        positions.dedup();
        positions
    }
}

impl Default for EditorSequenceTimeline {
    fn default() -> Self {
        Self::empty()
    }
}

impl gpui::Global for EditorSequenceTimeline {}

#[derive(Clone, Debug, PartialEq)]
pub struct SequenceRulerTick {
    pub position_millis: u32,
    pub fraction: f32,
    pub major: bool,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceTrackKind {
    Motion,
    EventSummary,
    Event,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceTrackProjection {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub depth: u32,
    pub kind: SequenceTrackKind,
    pub clip: Option<SequenceClipProjection>,
    pub keys: Vec<SequenceKeyProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceClipProjection {
    pub label: String,
    pub start_millis: u32,
    pub end_millis: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceKeyProjection {
    pub label: String,
    pub position_millis: u32,
}

/// Derive the fixed-density time ruler used by the div-based workbench.
#[must_use]
pub fn sequence_ruler_ticks(duration_millis: u32) -> Vec<SequenceRulerTick> {
    if duration_millis == 0 {
        return Vec::new();
    }
    (0..=RULER_DIVISIONS)
        .map(|index| {
            let fraction = kit::ratio(index, RULER_DIVISIONS);
            let position_millis = kit::scaled(fraction, duration_millis);
            SequenceRulerTick {
                position_millis,
                fraction,
                major: index % 5 == 0,
                label: format!("{:.2}s", kit::ratio(position_millis, 1_000)),
            }
        })
        .collect()
}

/// Convert a scrub fraction into a clamped timeline position.
#[must_use]
pub fn sequence_scrub_millis(fraction: f32, duration_millis: u32) -> u32 {
    kit::scaled(fraction, duration_millis)
}

fn sequence_tracks(
    motion: &EditorAnimationMotionData,
    duration_millis: u32,
) -> Vec<SequenceTrackProjection> {
    let summary_keys = motion
        .events
        .iter()
        .map(|event| SequenceKeyProjection {
            label: event.name.clone(),
            position_millis: event.time_millis.min(duration_millis),
        })
        .collect();
    let mut tracks = vec![
        SequenceTrackProjection {
            key: "motion".to_owned(),
            label: "Motion".to_owned(),
            detail: motion.name.clone(),
            depth: 0,
            kind: SequenceTrackKind::Motion,
            clip: Some(SequenceClipProjection {
                label: motion.name.clone(),
                start_millis: 0,
                end_millis: duration_millis,
            }),
            keys: Vec::new(),
        },
        SequenceTrackProjection {
            key: "events".to_owned(),
            label: "Events".to_owned(),
            detail: format!("{} keys", motion.events.len()),
            depth: 0,
            kind: SequenceTrackKind::EventSummary,
            clip: None,
            keys: summary_keys,
        },
    ];

    tracks.extend(motion.events.iter().enumerate().map(|(index, event)| {
        let start = event.time_millis.min(duration_millis);
        let end = event.end_time_millis.max(start).min(duration_millis);
        let mut keys = vec![SequenceKeyProjection {
            label: format!("{} start", event.name),
            position_millis: start,
        }];
        if end > start {
            keys.push(SequenceKeyProjection {
                label: format!("{} end", event.name),
                position_millis: end,
            });
        }
        SequenceTrackProjection {
            key: format!("event-{index}"),
            label: event.name.clone(),
            detail: event.parameter.clone(),
            depth: 1,
            kind: SequenceTrackKind::Event,
            clip: (end > start).then(|| SequenceClipProjection {
                label: event.parameter.clone(),
                start_millis: start,
                end_millis: end,
            }),
            keys,
        }
    }));
    tracks
}

fn transport_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    enabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(24.0))
        .min_w(px(30.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(if active {
            theme.primary.opacity(0.22)
        } else {
            theme.secondary
        })
        .font_family(theme.mono_font_family.clone())
        .text_size(px(10.0))
        .text_color(if !enabled {
            theme.muted_foreground.opacity(0.45)
        } else if active {
            theme.primary
        } else {
            theme.foreground
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.secondary_hover))
        })
        .child(label)
}

fn timecode(millis: u32) -> String {
    let minutes = millis / 60_000;
    let seconds = (millis / 1_000) % 60;
    let millis = millis % 1_000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

fn frame_number(millis: u32, fps: u32) -> u32 {
    u32::try_from((u64::from(millis) * u64::from(fps)) / 1_000).unwrap_or(u32::MAX)
}

fn render_track_row(
    track: &SequenceTrackProjection,
    row_index: usize,
    duration_millis: u32,
    progress: f32,
    theme: &Theme,
) -> gpui::AnyElement {
    let tone = match track.kind {
        SequenceTrackKind::Motion => theme.primary,
        SequenceTrackKind::EventSummary => theme.info,
        SequenceTrackKind::Event => theme.warning,
    };
    let lane_bg = if row_index.is_multiple_of(2) {
        theme.background
    } else {
        theme.sidebar.opacity(0.72)
    };
    let duration = duration_millis.max(1);

    h_flex()
        .id(format!("seq-track-row-{}", track.key))
        .flex_none()
        .h(px(TRACK_ROW_HEIGHT))
        .border_b_1()
        .border_color(theme.border)
        .child(render_track_name_cell(track, tone, theme))
        .child(render_track_lane(
            track, duration, progress, tone, lane_bg, theme,
        ))
        .into_any_element()
}

/// The fixed-width gutter cell: status dot, indented track label, and the
/// mono detail column.
fn render_track_name_cell(
    track: &SequenceTrackProjection,
    tone: gpui::Hsla,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .w(px(TRACK_NAME_WIDTH))
        .h_full()
        .gap(px(7.0))
        .pl(kit::indent(track.depth, 16.0, 10.0))
        .pr(px(8.0))
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .child(kit::status_dot(tone))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .overflow_hidden()
                .child(track.label.clone()),
        )
        .child(
            div()
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(9.5))
                .text_color(theme.muted_foreground)
                .child(track.detail.clone()),
        )
}

/// The scrolling lane: the clip bar, one diamond per key, and the playhead.
fn render_track_lane(
    track: &SequenceTrackProjection,
    duration: u32,
    progress: f32,
    tone: gpui::Hsla,
    lane_bg: gpui::Hsla,
    theme: &Theme,
) -> impl IntoElement {
    let clip = track.clip.clone();
    let keys = track.keys.clone();
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .relative()
        .bg(lane_bg)
        .children(clip.map(|clip| {
            let start = kit::ratio(clip.start_millis, duration);
            let width = kit::ratio(clip.end_millis.saturating_sub(clip.start_millis), duration);
            div()
                .absolute()
                .left(relative(start.clamp(0.0, 1.0)))
                .top(px(7.0))
                .h(px(20.0))
                .w(relative(width.clamp(0.0, 1.0)))
                .min_w(px(2.0))
                .px(px(5.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(tone.opacity(0.8))
                .bg(tone.opacity(0.18))
                .font_family(theme.mono_font_family.clone())
                .text_size(px(9.5))
                .text_color(tone)
                .overflow_hidden()
                .child(clip.label)
        }))
        .children(keys.into_iter().map(|key| {
            let fraction = kit::ratio(key.position_millis, duration);
            div()
                .absolute()
                .left(relative(fraction.clamp(0.0, 1.0)))
                .top(px(8.0))
                .ml(px(-4.0))
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(tone)
                .child("◆")
        }))
        .child(
            div()
                .absolute()
                .left(relative(progress))
                .top_0()
                .h_full()
                .w(px(1.0))
                .bg(theme.danger),
        )
}

/// Full-width Sequencer center workbench. It follows the proven motion-track
/// bounds hook + mouse scrub pattern and renders all lanes with GPUI divs.
pub struct SequencerWorkbenchPanel {
    focus_handle: FocusHandle,
    scrub_bounds: Bounds<Pixels>,
    scrubbing: bool,
}

impl SequencerWorkbenchPanel {
    pub const NAME: &'static str = "sequencer";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            scrub_bounds: Bounds::default(),
            scrubbing: false,
        }
    }

    fn scrub_to_position(
        &self,
        position: Point<Pixels>,
        duration_millis: u32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if duration_millis == 0 {
            return;
        }
        let width = f32::from(self.scrub_bounds.size.width).max(1.0);
        let x = f32::from(position.x - self.scrub_bounds.origin.x).clamp(0.0, width);
        window.dispatch_action(
            Box::new(SeqScrub {
                position_millis: sequence_scrub_millis(x / width, duration_millis),
            }),
            cx,
        );
        cx.notify();
    }
}

impl Render for SequencerWorkbenchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) = render_project_host_connection_placeholder("Sequencer", cx) {
            return placeholder;
        }

        let theme = cx.theme().clone();
        let timeline = cx
            .try_global::<EditorSequenceTimeline>()
            .cloned()
            .unwrap_or_default();
        let has_source = timeline.has_source();
        let duration = timeline.duration_millis;
        let progress = if duration == 0 {
            0.0
        } else {
            kit::ratio(timeline.position_millis, duration).clamp(0.0, 1.0)
        };
        let current_frame = frame_number(timeline.position_millis, timeline.fps);
        let duration_frames = frame_number(duration, timeline.fps);
        let track_count = timeline.tracks.len();
        let track_body = if has_source {
            v_flex()
                .size_full()
                .overflow_y_scrollbar()
                .children(timeline.tracks.iter().enumerate().map(|(index, track)| {
                    render_track_row(track, index, duration, progress, &theme)
                }))
                .into_any_element()
        } else {
            kit::empty_state(
                "No motion selected.",
                Some("Select a motion in Animation mode to project its event tracks.".to_owned()),
                &theme,
            )
            .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .bg(theme.background)
            .child(render_transport_bar(
                &timeline,
                has_source,
                current_frame,
                duration_frames,
                &theme,
            ))
            .child(render_ruler_strip(
                &timeline, has_source, duration, progress, &theme, cx,
            ))
            .child(v_flex().flex_1().min_h_0().child(track_body))
            .child(render_sequencer_footer(
                &timeline,
                duration_frames,
                track_count,
                &theme,
            ))
            .into_any_element()
    }
}

/// Transport bar: source dot, motion title, the six transport buttons, the
/// timecode/frame readout, the fps label, and the loop toggle.
fn render_transport_bar(
    timeline: &EditorSequenceTimeline,
    has_source: bool,
    current_frame: u32,
    duration_frames: u32,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .h(px(44.0))
        .items_center()
        .gap(px(7.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .child(kit::status_dot(if has_source {
            theme.primary
        } else {
            theme.muted_foreground
        }))
        .child(
            div()
                .max_w(px(190.0))
                .overflow_hidden()
                .text_size(px(11.5))
                .text_color(theme.foreground)
                .child(timeline.title.clone()),
        )
        .child(div().flex_none().w(px(1.0)).h(px(20.0)).bg(theme.border))
        .child(render_transport_buttons(timeline, has_source, theme))
        .child(
            h_flex()
                .h(px(26.0))
                .gap(px(7.0))
                .px(px(9.0))
                .rounded(px(5.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.5))
                .text_color(theme.foreground)
                .child(timecode(timeline.position_millis))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("f{current_frame} / {duration_frames}")),
                ),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(format!("{} fps", timeline.fps)),
        )
        .child(
            transport_button(
                "seq-loop",
                if timeline.looping { "LOOP" } else { "ONCE" },
                timeline.looping,
                has_source,
                theme,
            )
            .ml_auto()
            .on_mouse_down(MouseButton::Left, {
                let looping = timeline.looping;
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(Box::new(SeqSetLoop { looping: !looping }), cx);
                    }
                }
            }),
        )
}

/// The six transport buttons — start, previous key, play/pause, stop, next
/// key, end — each a no-op while no motion is selected.
fn render_transport_buttons(
    timeline: &EditorSequenceTimeline,
    has_source: bool,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .gap(px(3.0))
        .child(
            transport_button("seq-start", "|<", false, has_source, theme).on_mouse_down(
                MouseButton::Left,
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(
                            Box::new(SeqGoToKey {
                                target: SeqKeyTarget::Start,
                            }),
                            cx,
                        );
                    }
                },
            ),
        )
        .child(
            transport_button("seq-previous-key", "<", false, has_source, theme).on_mouse_down(
                MouseButton::Left,
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(
                            Box::new(SeqGoToKey {
                                target: SeqKeyTarget::Previous,
                            }),
                            cx,
                        );
                    }
                },
            ),
        )
        .child(
            transport_button(
                "seq-play",
                if timeline.playing { "PAUSE" } else { "PLAY" },
                timeline.playing,
                has_source,
                theme,
            )
            .on_mouse_down(MouseButton::Left, {
                let playing = timeline.playing;
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(Box::new(SeqPlay { playing: !playing }), cx);
                    }
                }
            }),
        )
        .child(
            transport_button("seq-stop", "STOP", false, has_source, theme).on_mouse_down(
                MouseButton::Left,
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(Box::new(SeqStop), cx);
                    }
                },
            ),
        )
        .child(
            transport_button("seq-next-key", ">", false, has_source, theme).on_mouse_down(
                MouseButton::Left,
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(
                            Box::new(SeqGoToKey {
                                target: SeqKeyTarget::Next,
                            }),
                            cx,
                        );
                    }
                },
            ),
        )
        .child(
            transport_button("seq-end", ">|", false, has_source, theme).on_mouse_down(
                MouseButton::Left,
                move |_, window, cx| {
                    if has_source {
                        window.dispatch_action(
                            Box::new(SeqGoToKey {
                                target: SeqKeyTarget::End,
                            }),
                            cx,
                        );
                    }
                },
            ),
        )
}

/// Time ruler strip: the TRACKS gutter label beside the scrubbable ruler, its
/// tick markers, and the playhead line the transport drives.
fn render_ruler_strip(
    timeline: &EditorSequenceTimeline,
    has_source: bool,
    duration: u32,
    progress: f32,
    theme: &Theme,
    cx: &Context<'_, SequencerWorkbenchPanel>,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .h(px(30.0))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .child(
            h_flex()
                .flex_none()
                .w(px(TRACK_NAME_WIDTH))
                .h_full()
                .px(px(10.0))
                .border_r_1()
                .border_color(theme.border)
                .text_size(px(9.5))
                .text_color(theme.muted_foreground)
                .child("TRACKS"),
        )
        .child(
            div()
                .id("seq-ruler")
                .flex_1()
                .min_w_0()
                .h_full()
                .relative()
                .cursor_pointer()
                .on_prepaint({
                    let entity = cx.entity();
                    move |bounds, _, cx| {
                        entity.update(cx, |this, _| {
                            this.scrub_bounds = bounds;
                        });
                    }
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.scrubbing = has_source;
                        this.scrub_to_position(event.position, duration, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                        if this.scrubbing {
                            this.scrub_to_position(event.position, duration, window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.scrubbing = false;
                        cx.notify();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.scrubbing = false;
                        cx.notify();
                    }),
                )
                .children(
                    timeline
                        .ruler_ticks
                        .iter()
                        .map(|tick| render_ruler_tick(tick, theme)),
                )
                .child(
                    div()
                        .absolute()
                        .left(relative(progress))
                        .top_0()
                        .h_full()
                        .w(px(1.0))
                        .bg(theme.danger),
                )
                .child(
                    div()
                        .absolute()
                        .left(relative(progress))
                        .top(px(-4.0))
                        .ml(px(-5.0))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .text_color(theme.danger)
                        .child("▼"),
                ),
        )
}

/// One ruler tick: a short line, and for a major tick the timecode label
/// floated above it.
fn render_ruler_tick(tick: &SequenceRulerTick, theme: &Theme) -> impl IntoElement {
    let mut marker = div()
        .absolute()
        .left(relative(tick.fraction))
        .bottom_0()
        .w(px(1.0))
        .h(px(if tick.major { 10.0 } else { 5.0 }))
        .bg(if tick.major {
            theme.muted_foreground
        } else {
            theme.border
        });
    if tick.major {
        marker = marker.child(
            div()
                .absolute()
                .left(px(3.0))
                .top(px(-14.0))
                .font_family(theme.mono_font_family.clone())
                .text_size(px(9.0))
                .text_color(theme.muted_foreground)
                .child(tick.label.clone()),
        );
    }
    marker
}

/// Footer summary: motion title, frame and fps counts, track count, and the
/// read-only notice.
fn render_sequencer_footer(
    timeline: &EditorSequenceTimeline,
    duration_frames: u32,
    track_count: usize,
    theme: &Theme,
) -> impl IntoElement {
    kit::count_footer(theme)
        .child(timeline.title.clone())
        .child(format!("{duration_frames} frames"))
        .child(format!("{} fps", timeline.fps))
        .child(format!("{track_count} tracks"))
        .child(
            div()
                .ml_auto()
                .text_color(theme.muted_foreground)
                .child("motion events · read-only"),
        )
}

impl Focusable for SequencerWorkbenchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SequencerWorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("movie"), "Sequencer", kit::TabTone::Muted)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<PanelEvent> for SequencerWorkbenchPanel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_ticks_cover_the_timeline_with_major_quarters() {
        let ticks = sequence_ruler_ticks(4_000);

        assert_eq!(ticks.len(), 21);
        assert_eq!(ticks.first().map(|tick| tick.position_millis), Some(0));
        assert_eq!(ticks.last().map(|tick| tick.position_millis), Some(4_000));
        assert_eq!(
            ticks
                .iter()
                .filter(|tick| tick.major)
                .map(|tick| tick.position_millis)
                .collect::<Vec<_>>(),
            vec![0, 1_000, 2_000, 3_000, 4_000]
        );
    }

    #[test]
    fn scrub_fraction_maps_to_clamped_millis() {
        assert_eq!(sequence_scrub_millis(0.375, 2_000), 750);
        assert_eq!(sequence_scrub_millis(-0.5, 2_000), 0);
        assert_eq!(sequence_scrub_millis(1.5, 2_000), 2_000);
    }
}
