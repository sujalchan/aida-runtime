struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) line_position: vec2<f32>,
    @location(1) line_length: f32,
    @location(2) half_width: f32,
    @location(3) color: vec4<f32>,
};

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

fn quad_corner(vertex_index: u32) -> vec2<f32> {
    switch vertex_index {
        case 0u: { return vec2<f32>(0.0, 0.0); }
        case 1u: { return vec2<f32>(0.0, 1.0); }
        case 2u: { return vec2<f32>(1.0, 0.0); }
        case 3u: { return vec2<f32>(1.0, 0.0); }
        case 4u: { return vec2<f32>(0.0, 1.0); }
        default: { return vec2<f32>(1.0, 1.0); }
    }
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) diameter: f32,
    @location(4) pixel_offset: f32,
) -> VertexOutput {
    let aligned_start = start + vec2<f32>(pixel_offset);
    let aligned_end = end + vec2<f32>(pixel_offset);
    let delta = aligned_end - aligned_start;
    let line_length = length(delta);
    let half_width = max(diameter, 1.0) * 0.5;
    let direction = select(vec2<f32>(1.0, 0.0), delta / max(line_length, 1e-5), line_length > 1e-5);
    let normal = vec2<f32>(-direction.y, direction.x);
    let corner = quad_corner(vertex_index);
    // leave a small raster margin so the antialiased circle edge is not cut by the quad.
    let raster_margin = 1.0;
    let along = mix(-half_width - raster_margin, line_length + half_width + raster_margin, corner.x);
    let across = mix(-half_width - raster_margin, half_width + raster_margin, corner.y);
    let stage_position = aligned_start + direction * along + normal * across;

    var output: VertexOutput;
    output.position = vec4<f32>(
        stage_position.x * (2.0 / 480.0),
        stage_position.y * (2.0 / 360.0),
        0.0,
        1.0,
    );
    output.line_position = vec2<f32>(along, across);
    output.line_length = line_length;
    output.half_width = half_width;
    output.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    return output;
}

@vertex
fn vs_dot(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) diameter: f32,
    @location(3) pixel_offset: f32,
) -> VertexOutput {
    let half_width = max(diameter, 1.0) * 0.5;
    let corner = quad_corner(vertex_index);
    let raster_margin = 1.0;
    let along = mix(-half_width - raster_margin, half_width + raster_margin, corner.x);
    let across = mix(-half_width - raster_margin, half_width + raster_margin, corner.y);
    let point_position = position + vec2<f32>(pixel_offset);
    let stage_position = point_position + vec2<f32>(along, across);

    var output: VertexOutput;
    output.position = vec4<f32>(
        stage_position.x * (2.0 / 480.0),
        stage_position.y * (2.0 / 360.0),
        0.0,
        1.0,
    );
    output.line_position = vec2<f32>(along, across);
    output.line_length = 0.0;
    output.half_width = half_width;
    output.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let closest_along = clamp(input.line_position.x, 0.0, input.line_length);
    let distance = length(vec2<f32>(
        input.line_position.x - closest_along,
        input.line_position.y,
    )) - input.half_width;
    let pixel_footprint = max(fwidth(distance), 1e-4);
    let coverage = 1.0 - smoothstep(-pixel_footprint * 0.5, pixel_footprint * 0.5, distance);
    let alpha = input.color.a * coverage;
    return vec4<f32>(input.color.rgb * alpha, alpha);
}
