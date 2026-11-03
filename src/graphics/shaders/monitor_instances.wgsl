struct In {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) shape: vec4<f32>,
    @location(5) monitor_id: u32,
};
@group(0) @binding(0) var<storage, read> monitor_transforms: array<vec4<f32>>;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec2<f32>,
    @location(5) shape: vec4<f32>,
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
    out.shape = input.shape;
    if (out.shape.x <= 0.0 || out.shape.y <= 0.0) {
        out.shape = vec4(input.rect.zw, 0.0, 0.0);
    }
    out.shape.x *= transform.z;
    out.shape.y *= transform.z;
    out.shape.w *= transform.z;
    return out;
}
fn rounded_box(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let b = size * 0.5;
    let r = clamp(radius, 0.0, min(b.x, b.y));
    let q = abs(p - b) - b + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
fn tail_curve(t: f32, start: vec2<f32>, control_a: vec2<f32>, control_b: vec2<f32>, tip: vec2<f32>) -> vec2<f32> {
    let u = 1.0 - t;
    return u * u * u * start
        + 3.0 * u * u * t * control_a
        + 3.0 * u * t * t * control_b
        + t * t * t * tip;
}
fn speech_tail_distance(p: vec2<f32>, body_size: vec2<f32>, anchor_x: f32, direction: f32) -> f32 {
    let tail_scale = 0.55;
    let attachment = vec2(anchor_x, body_size.y - 4.0);
    let start = attachment + vec2(0.0, -1.0) * tail_scale;
    let tip = attachment + vec2(12.0 * direction, 22.0) * tail_scale;
    let tangent = vec2(4.0 * direction, 8.0) * tail_scale;
    let control_a = start + tangent;
    let control_b = tip - tangent;
    var distance = 100000.0;
    for (var index = 0u; index < 32u; index += 1u) {
        let t0 = f32(index) / 32.0;
        let t1 = f32(index + 1u) / 32.0;
        let a = tail_curve(t0, start, control_a, control_b, tip);
        let b = tail_curve(t1, start, control_a, control_b, tip);
        let segment = b - a;
        let projection = clamp(dot(p - a, segment) / max(dot(segment, segment), 0.0001), 0.0, 1.0);
        let t = mix(t0, t1, projection);
        let center = a + segment * projection;
        let radius = (3.4 + 8.6 * pow(max(1.0 - t, 0.0), 1.08)) * tail_scale;
        distance = min(distance, length(p - center) - radius);
    }
    return distance;
}
fn smooth_union(a: f32, b: f32, radius: f32) -> f32 {
    let h = max(radius - abs(a - b), 0.0) / radius;
    return min(a, b) - h * h * radius * 0.25;
}
@fragment fn fs_main(input: Out) -> @location(0) vec4<f32> {
    var d = rounded_box(input.local, input.shape.xy, input.params.x);
    if (input.shape.z > 0.5) {
        let direction = select(-1.0, 1.0, input.shape.z > 1.5);
        d = smooth_union(
            d,
            speech_tail_distance(input.local, input.shape.xy, input.shape.w, direction),
            3.0,
        );
    }
    let aa = max(fwidth(d), 0.5);
    let outer = 1.0 - smoothstep(-aa, aa, d);
    if (input.params.y <= 0.0) { return input.fill * outer; }
    let inner = 1.0 - smoothstep(-aa, aa, d + input.params.y);
    return input.fill * inner + input.border * max(outer - inner, 0.0);
}
