// Newly authored, single-pass presentation effects. Both frontends append this
// exact source. Effect colors and interpolation use the encoded sRGB domain;
// wrappers handle the target format without changing their legacy Off paths.
fn display_linear(rgb: vec3<f32>) -> vec3<f32> {
    return select(rgb / 12.92, pow((rgb + 0.055) / 1.055, vec3(2.4)), rgb > vec3(0.04045));
}

fn display_uv(uv: vec2<f32>, preset: u32) -> vec2<f32> {
    if preset != 3u {
        return uv;
    }
    let centered = uv * 2.0 - 1.0;
    let bowed = centered * (1.0 + 0.08 * centered.yx * centered.yx);
    return bowed * 0.5 + 0.5;
}

fn display_sample_uv(uv: vec2<f32>, size: vec2<f32>, preset: u32, sharp: bool) -> vec2<f32> {
    if preset != 1u && !sharp {
        return uv;
    }
    // Smooth widens sharp-bilinear seams to two destination pixels, retaining
    // flat interiors (linear at <=2x). CRT retains the chosen sampling filter.
    let texel = uv * size;
    // Continuous footprint avoids integer-threshold seams on a curved surface
    // and is stable across encoded/sRGB target compiler variants. The faithful
    // Off wrappers retain their original integer-prescale expression.
    let softness = select(1.0, 0.5, preset == 1u);
    let prescale = max(softness / fwidth(texel), vec2(1.0));
    let region = 0.5 - 0.5 / prescale;
    let offset = fract(texel) - 0.5;
    return (floor(texel) + 0.5 + (offset - clamp(offset, -region, region)) * prescale) / size;
}

fn display_rgb(color: vec3<f32>, uv: vec2<f32>, position: vec2<f32>, size: vec2<f32>, preset: u32) -> vec3<f32> {
    if preset < 2u {
        return color;
    }
    let curved = preset == 3u;
    // Fade scanlines below 2x, preventing temporal shimmer/moire at 1x or less.
    let line_scale = 1.0 / max(fwidth(uv.y) * size.y, 0.0001);
    let line_strength = select(0.20, 0.28, curved) * clamp(line_scale - 1.0, 0.0, 1.0);
    let scanline = 1.0 - line_strength * (0.5 + 0.5 * cos(uv.y * size.y * 6.28318530718));
    // One physical-pixel RGB aperture triad, independent of pixel aspect/scale.
    let mask_strength = select(0.12, 0.18, curved);
    let phase = u32(floor(position.x)) % 3u;
    let mask = vec3(select(1.0 - mask_strength, 1.0, phase == 0u),
                    select(1.0 - mask_strength, 1.0, phase == 1u),
                    select(1.0 - mask_strength, 1.0, phase == 2u));
    let centered = uv * 2.0 - 1.0;
    let vignette = select(1.0, 1.0 - 0.10 * dot(centered, centered), curved);
    let inside = all(uv >= vec2(0.0)) && all(uv <= vec2(1.0));
    return select(vec3(0.0), color * scanline * mask * vignette, inside);
}
