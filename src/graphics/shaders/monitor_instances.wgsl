struct In {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) monitor_id: u32,
};
@group(0) @binding(0) var<storage, read> monitor_transforms: array<vec4<f32>>;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec2<f32>,
};
@vertex fn vs_main(input: In, @builtin(vertex_index) vi: u32) -> Out {
    var corners = array<vec2<f32>, 6>(vec2(0.0,0.0),vec2(1.0,0.0),vec2(0.0,1.0),vec2(0.0,1.0),vec2(1.0,0.0),vec2(1.0,1.0));
    let uv = corners[vi];
    let transform = monitor_transforms[input.monitor_id];
    let pixel = transform.xy + (input.rect.xy + uv * input.rect.zw) * transform.z;
    var out: Out;
    out.position = vec4(2.0 * pixel.x / input.params.z - 1.0, 1.0 - 2.0 * pixel.y / input.params.w, 0.0, 1.0);
    out.local = uv * input.rect.zw * transform.z;
    out.size = input.rect.zw * transform.z;
    out.fill = input.fill;
    out.border = input.border;
    out.params = input.params.xy * transform.z;
    return out;
}
fn rounded_box(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let b = size * 0.5;
    let r = clamp(radius, 0.0, min(b.x, b.y));
    let q = abs(p - b) - b + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
@fragment fn fs_main(input: Out) -> @location(0) vec4<f32> {
    let d = rounded_box(input.local, input.size, input.params.x);
    let aa = max(fwidth(d), 0.5);
    let outer = 1.0 - smoothstep(-aa, aa, d);
    if (input.params.y <= 0.0) { return input.fill * outer; }
    let inner_size = max(input.size - vec2(input.params.y * 2.0), vec2(0.0));
    let inner_local = input.local - vec2(input.params.y);
    let inner_d = rounded_box(inner_local, inner_size, max(input.params.x - input.params.y, 0.0));
    let inner = 1.0 - smoothstep(-aa, aa, inner_d);
    return input.fill * inner + input.border * max(outer - inner, 0.0);
}
