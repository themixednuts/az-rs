//! Per-editor-mode side/workbench panels.
//!
//! Each non-scene activity-rail mode (materials, scripting, game data,
//! sequencer, animation) owns a left/right nav panel pair plus a center
//! workbench panel. These stay thin: domain data is published as GPUI
//! globals by editor-core, panels only project it. Chrome is built from the
//! shared kit (`panels::kit`) rather than hand-rolled styling.

pub mod animation;
pub mod gamedata;
pub mod materials;
pub mod scripting;
pub mod sequencer;

pub use animation::{
    AnimationMannequinPanel, AnimationMotionTrackPanel, AnimationNavigatorPanel,
    AnimationWorkbenchPanel,
};
pub use gamedata::{GameDataCatalogPanel, GameDataDetailsPanel, GameDataWorkbenchPanel};
pub use materials::{MaterialPalettePanel, MaterialParamsPanel, MaterialWorkbenchPanel};
pub use scripting::{ScriptEditorPanel, ScriptFilesPanel, ScriptOutlinePanel};
pub use sequencer::SequencerWorkbenchPanel;

/// A mode dock panel (side nav panel or center workbench) whose body is a
/// single kit-styled projection of GPUI globals. Renders the connecting
/// placeholder while the project host is not yet attached.
macro_rules! mode_panel {
    ($type:ident, $name:expr, $title:expr, $render:ident $(, $icon:expr, $tone:expr)?) => {
        pub struct $type {
            focus_handle: gpui::FocusHandle,
        }

        impl $type {
            pub const NAME: &'static str = $name;

            pub fn init(cx: &mut gpui::Context<'_, Self>) -> Self {
                Self {
                    focus_handle: cx.focus_handle(),
                }
            }
        }

        impl gpui::Render for $type {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                cx: &mut gpui::Context<'_, Self>,
            ) -> impl gpui::IntoElement {
                use gpui::IntoElement as _;
                if let Some(placeholder) =
                    crate::panels::render_project_host_connection_placeholder($title, cx)
                {
                    return placeholder;
                }
                $render(cx).into_any_element()
            }
        }

        impl gpui::Focusable for $type {
            fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
                self.focus_handle.clone()
            }
        }

        impl gpui_component::dock::Panel for $type {
            fn panel_name(&self) -> &'static str {
                Self::NAME
            }

            fn title(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<'_, Self>,
            ) -> impl gpui::IntoElement {
                mode_panel!(@title $title $(, $icon, $tone)?)
            }

            fn inner_padding(&self, _cx: &gpui::App) -> bool {
                false
            }
        }

        impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for $type {}
    };
    (@title $title:expr) => {
        $title
    };
    (@title $title:expr, $icon:expr, $tone:expr) => {
        crate::panels::kit::tab_title($icon, $title, $tone)
    };
}

pub(crate) use mode_panel;
