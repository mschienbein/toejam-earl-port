@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
override encode_srgb: bool = false;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    let position = positions[index];
    var out: Vertex;
    out.position = vec4(position, 0.0, 1.0);
    out.uv = vec2((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return out;
}

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    let color = textureSample(frame, frame_sampler, vertex.uv);
    if encode_srgb {
        let rgb = select(12.92 * color.rgb, 1.055 * pow(color.rgb, vec3(1.0 / 2.4)) - 0.055, color.rgb > vec3(0.0031308));
        return vec4(rgb, 1.0);
    }
    return vec4(color.rgb, 1.0);
}
