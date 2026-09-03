use crate::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Style, StyleRefinement, Styled, Window,
};
use refineable::Refineable;

/// A layout element whose paint operation reveals a lower
/// DirectComposition visual at this exact scene order.
pub struct CompositionHoleElement {
    slot_id: u64,
    scene_generation: u64,
    corner_radii: Corners<Pixels>,
    style: StyleRefinement,
}

/// Construct a typed composition hole for a renderer-owned visual slot.
pub fn composition_hole(slot_id: u64, scene_generation: u64) -> CompositionHoleElement {
    CompositionHoleElement {
        slot_id,
        scene_generation,
        corner_radii: Corners::default(),
        style: StyleRefinement::default(),
    }
}

impl CompositionHoleElement {
    /// Set the four logical-pixel corner radii used by both the top-surface
    /// erase and the lower visual clip.
    pub fn corner_radii(mut self, corner_radii: Corners<Pixels>) -> Self {
        self.corner_radii = corner_radii;
        self
    }
}

impl Element for CompositionHoleElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.paint_composition_hole(
            bounds,
            self.corner_radii,
            self.slot_id,
            self.scene_generation,
        );
    }
}

impl IntoElement for CompositionHoleElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for CompositionHoleElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
