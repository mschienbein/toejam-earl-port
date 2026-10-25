struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var output: VertexOutput;
    output.position = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    output.uv = vec2<f32>(x, 1.0 - y);
    return output;
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(0) @binding(2) var<uniform> options: vec4<u32>;
override target_srgb: bool = false;
@fragment fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let preset = options.y;
    let source_uv = display_uv(input.uv, preset);
    var uv = source_uv;
    if preset != 0u {
        uv = display_sample_uv(uv, vec2<f32>(textureDimensions(image)), preset, options.x == 0u);
    } else if options.x == 0u {
        // Sharp-bilinear: preserve flat texel interiors, blending only at an
        // edge's destination-pixel footprint. No intermediate texture/pass.
        let dimensions = vec2<f32>(textureDimensions(image));
        let pixel = uv * dimensions;
        let prescale = max(floor(1.0 / fwidth(pixel) + 0.0001), vec2<f32>(1.0));
        let region = 0.5 - 0.5 / prescale;
        let offset = fract(pixel) - 0.5;
        uv = (floor(pixel) + 0.5 + (offset - clamp(offset, -region, region)) * prescale) / dimensions;
    }
    let color = textureSample(image, image_sampler, uv);
    if preset == 0u {
        return color;
    }
    let rgb = display_rgb(color.rgb, source_uv, input.position.xy,
        vec2<f32>(textureDimensions(image)), preset);
    if target_srgb {
        return vec4(display_linear(rgb), 1.0);
    }
    return vec4(rgb, 1.0);
}
