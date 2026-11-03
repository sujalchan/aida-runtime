@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
struct UvOffset {
    value: vec2<f32>,
    shadow_opacity: f32,
    _padding: f32,
};
@group(0) @binding(2) var<uniform> uv_offset: UvOffset;
@group(0) @binding(3) var shadow_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // one large triangle covers the active viewport without a vertex buffer.
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
    let layer = textureSample(layer_texture, layer_sampler, uv);
    let layer_alpha = layer.a;
    if uv_offset.shadow_opacity <= 0.0 {
        return vec4<f32>(layer.rgb * layer_alpha, layer_alpha);
    }

    let shadow = textureSample(shadow_texture, layer_sampler, uv);
    let shadow_alpha = shadow.a * uv_offset.shadow_opacity;
    let alpha = layer_alpha + shadow_alpha * (1.0 - layer_alpha);
    let color = layer.rgb * layer_alpha
        + shadow.rgb * shadow_alpha * (1.0 - layer_alpha);
    return vec4<f32>(color, alpha);
}
