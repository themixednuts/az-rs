//! Material color helpers.

use bevy::color::Srgba;

#[must_use]
pub const fn material_color(red: f32, green: f32, blue: f32, alpha: f32) -> Srgba {
    Srgba::new(red, green, blue, alpha)
}

#[must_use]
pub fn material_color_from_native(value: &str) -> Option<Srgba> {
    let mut values = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f32>);

    let red = values.next()?.ok()?;
    let green = values.next()?.ok()?;
    let blue = values.next()?.ok()?;
    let alpha = values.next().transpose().ok()?.unwrap_or(1.0);

    Some(material_color(red, green, blue, alpha))
}
