@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
struct UvOffset {
    value: vec2<f32>,
    _shadow_opacity: f32,
    _padding: f32,
};
@group(0) @binding(2) var<uniform> uv_offset: UvOffset;
@group(0) @binding(3) var _shadow_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv - uv_offset.value;
    // the pen layer is stored premultiplied because it is incrementally drawn
    // with source-over blending. keep it premultiplied for the surface blend.
    return textureSample(layer_texture, layer_sampler, uv);
}
