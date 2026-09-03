//! Editor-owned scene tool state shared by the Aether toolbar and viewport.

use gpui::Global;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditorSceneToolKind {
    Select,
    #[default]
    Move,
    Rotate,
    Scale,
    Transform,
}

impl EditorSceneToolKind {
    pub const ALL: [Self; 5] = [
        Self::Select,
        Self::Move,
        Self::Rotate,
        Self::Scale,
        Self::Transform,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Move => "move",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
            Self::Transform => "transform",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
            Self::Transform => "Transform",
        }
    }

    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Select => "arrow_selector_tool",
            Self::Move => "open_with",
            Self::Rotate => "3d_rotation",
            Self::Scale => "open_in_full",
            Self::Transform => "transform",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Select => "Select (Q)",
            Self::Move => "Move (W)",
            Self::Rotate => "Rotate (E)",
            Self::Scale => "Scale (R)",
            Self::Transform => "Transform (T)",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.key() == key)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditorScenePivot {
    #[default]
    Pivot,
    Center,
}

impl EditorScenePivot {
    pub const ALL: [Self; 2] = [Self::Pivot, Self::Center];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Pivot => "pivot",
            Self::Center => "center",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pivot => "Pivot",
            Self::Center => "Center",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pivot| pivot.key() == key)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditorSceneTransformSpace {
    #[default]
    World,
    Local,
}

impl EditorSceneTransformSpace {
    pub const ALL: [Self; 2] = [Self::World, Self::Local];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Local => "local",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::World => "World",
            Self::Local => "Local",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|space| space.key() == key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorGridSnapState {
    pub enabled: bool,
    pub step_meters: f32,
}

impl Default for EditorGridSnapState {
    fn default() -> Self {
        Self {
            enabled: true,
            step_meters: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorAngleSnapState {
    pub enabled: bool,
    pub degrees: f32,
}

impl Default for EditorAngleSnapState {
    fn default() -> Self {
        Self {
            enabled: true,
            degrees: 15.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorSceneToolState {
    pub tool: EditorSceneToolKind,
    pub pivot: EditorScenePivot,
    pub space: EditorSceneTransformSpace,
    pub grid_snap: EditorGridSnapState,
    pub angle_snap: EditorAngleSnapState,
}

impl Global for EditorSceneToolState {}

impl EditorSceneToolState {
    pub const fn set_tool(&mut self, tool: EditorSceneToolKind) {
        self.tool = tool;
    }

    pub const fn set_pivot(&mut self, pivot: EditorScenePivot) {
        self.pivot = pivot;
    }

    pub const fn set_space(&mut self, space: EditorSceneTransformSpace) {
        self.space = space;
    }

    pub const fn toggle_grid_snap(&mut self) {
        self.grid_snap.enabled = !self.grid_snap.enabled;
    }

    pub const fn toggle_angle_snap(&mut self) {
        self.angle_snap.enabled = !self.angle_snap.enabled;
    }

    #[must_use]
    pub fn grid_step_label(&self) -> String {
        format_decimal(self.grid_snap.step_meters, "m")
    }

    #[must_use]
    pub fn angle_degrees_label(&self) -> String {
        format_decimal(self.angle_snap.degrees, "")
    }
}

fn format_decimal(value: f32, suffix: &str) -> String {
    if (value.fract()).abs() < f32::EPSILON {
        format!("{value:.0}{suffix}")
    } else {
        format!("{value:.1}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_matches_toolbar_defaults() {
        let state = EditorSceneToolState::default();

        assert_eq!(state.tool, EditorSceneToolKind::Move);
        assert_eq!(state.pivot, EditorScenePivot::Pivot);
        assert_eq!(state.space, EditorSceneTransformSpace::World);
        assert!(state.grid_snap.enabled);
        assert_eq!(state.grid_step_label(), "0.5m");
        assert!(state.angle_snap.enabled);
        assert_eq!(state.angle_degrees_label(), "15");
    }

    #[test]
    fn transitions_update_scene_tool_state() {
        let mut state = EditorSceneToolState::default();

        state.set_tool(EditorSceneToolKind::Rotate);
        state.set_pivot(EditorScenePivot::Center);
        state.set_space(EditorSceneTransformSpace::Local);
        state.toggle_grid_snap();
        state.toggle_angle_snap();

        assert_eq!(state.tool, EditorSceneToolKind::Rotate);
        assert_eq!(state.pivot, EditorScenePivot::Center);
        assert_eq!(state.space, EditorSceneTransformSpace::Local);
        assert!(!state.grid_snap.enabled);
        assert!(!state.angle_snap.enabled);
    }
}
