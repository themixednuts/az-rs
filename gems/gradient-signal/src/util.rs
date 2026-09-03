//! `GradientSignal` math helpers.

/// Applies `GradientSignal` levels remapping.
///
/// `input_gamma` is `inputMid` in the source: the midtone exponent the remapped
/// input is raised to.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/Util.h:101`.
#[must_use]
pub fn levels_value(
    input: f32,
    input_gamma: f32,
    input_min: f32,
    input_max: f32,
    output_min: f32,
    output_max: f32,
) -> f32 {
    let input = input.clamp(0.0, 1.0);
    let input_gamma = input_gamma.clamp(0.01, 10.0);
    let input_min = input_min.clamp(0.0, 1.0);
    let input_max = input_max.clamp(0.0, 1.0);
    let output_min = output_min.clamp(0.0, 1.0);
    let output_max = output_max.clamp(0.0, 1.0);

    let input_range = input_max - input_min;
    let input_corrected = if input_range == 0.0 {
        if input <= input_min { 0.0 } else { 1.0 }
    } else {
        ((input - input_min).max(0.0) / input_range)
            .min(1.0)
            .powf(1.0 / input_gamma)
    };

    (output_max - output_min).mul_add(input_corrected, output_min)
}
