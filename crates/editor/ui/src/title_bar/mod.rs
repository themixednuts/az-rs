
use gpui::{Context, IntoElement, Render, Window, div};

pub struct TitleBar;

impl TitleBar {
	pub fn new(_window: &mut Window, _cx: &mut Context<'_, Self>) -> Self {
		Self
	}
}

impl Render for TitleBar {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
		div()
	}
}


