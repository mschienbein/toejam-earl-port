// Faithful presenter at integrate/extras 9f4c055, retained as an independent pixel oracle.
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
@fragment fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    var uv = input.uv;
    if options.x == 0u {
        let dimensions = vec2<f32>(textureDimensions(image));
        let pixel = uv * dimensions;
        let prescale = max(floor(1.0 / fwidth(pixel) + 0.0001), vec2<f32>(1.0));
        let region = 0.5 - 0.5 / prescale;
        let offset = fract(pixel) - 0.5;
        uv = (floor(pixel) + 0.5 + (offset - clamp(offset, -region, region)) * prescale) / dimensions;
    }
    return textureSample(image, image_sampler, uv);
}
