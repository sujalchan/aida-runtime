struct In { @location(0) local_position: vec2<f32>, @location(1) glyph_id: u32, @location(2) monitor_id: u32, @location(3) color: vec4<f32>, @location(4) scale: f32, @location(5) clip_y: vec2<f32> };
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32>, @location(2) stage_position: vec2<f32>, @location(3) clip_y: vec2<f32> };
struct GlyphMeta { uv: vec4<f32>, bounds: vec4<f32> };
@group(0) @binding(0) var<storage, read> monitor_transforms: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> glyph_metadata: array<GlyphMeta>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;
@vertex fn vs_main(input: In, @builtin(vertex_index) vi: u32) -> Out {
    var corners = array<vec2<f32>, 6>(vec2(0.0,0.0),vec2(1.0,0.0),vec2(0.0,1.0),vec2(0.0,1.0),vec2(1.0,0.0),vec2(1.0,1.0));
    let p = corners[vi];
    let transform = monitor_transforms[input.monitor_id];
    let glyph_data = glyph_metadata[input.glyph_id];
    let pixel = transform.xy + (input.local_position + (glyph_data.bounds.xy + p * glyph_data.bounds.zw) * input.scale) * transform.z;
    var out: Out;
    out.position = vec4(2.0 * pixel.x / 480.0 - 1.0, 1.0 - 2.0 * pixel.y / 360.0, 0.0, 1.0);
    out.uv = glyph_data.uv.xy + p * glyph_data.uv.zw;
    out.color = input.color;
    out.stage_position = pixel;
    out.clip_y = vec2(0.0);
    if (input.clip_y.y > input.clip_y.x) {
        let clip_min = transform.y + input.clip_y.x * transform.z;
        let clip_max = transform.y + input.clip_y.y * transform.z;
        out.clip_y = vec2(clip_min, clip_max);
    }
    return out;
}
@fragment fn fs_main(input: Out) -> @location(0) vec4<f32> {
    if (input.clip_y.y > input.clip_y.x && (input.stage_position.y < input.clip_y.x || input.stage_position.y >= input.clip_y.y)) { discard; }
    let alpha = textureSample(atlas, atlas_sampler, input.uv).r;
    return vec4(input.color.rgb * input.color.a * alpha, input.color.a * alpha);
}
