struct SpriteTransform {
    position: vec2<f32>,
    logical_size: vec2<f32>,
    rotation_center: vec2<f32>,
    direction: f32,
    scale: f32,
    visible: f32,
    flip_x: f32,
    _padding1: f32,
    _padding2: f32,
    effects_0: vec4<f32>,
    effects_1: vec4<f32>,
};

@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;
@group(1) @binding(0) var<uniform> transform: SpriteTransform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) effects_0: vec4<f32>,
    @location(2) effects_1: vec4<f32>,
};

const EFFECT_EPSILON: f32 = 1e-3;

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

fn linear_to_srgb(color: vec3<f32>) -> vec3<f32> {
    let bounded = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = bounded * 12.92;
    let high = 1.055 * pow(bounded, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, bounded <= vec3<f32>(0.0031308));
}

fn rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let hue_offsets = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let temp1 = select(
        vec4<f32>(rgb.bg, hue_offsets.wz),
        vec4<f32>(rgb.gb, hue_offsets.xy),
        rgb.b <= rgb.g,
    );
    let temp2 = select(
        vec4<f32>(temp1.xyw, rgb.r),
        vec4<f32>(rgb.r, temp1.yzx),
        rgb.r > temp1.x,
    );
    let minimum = min(temp2.y, temp2.w);
    let chroma = temp2.x - minimum;
    return vec3<f32>(
        abs(temp2.z + (temp2.w - temp2.y) / (6.0 * chroma + EFFECT_EPSILON)),
        chroma / (temp2.x + EFFECT_EPSILON),
        temp2.x,
    );
}

fn hue_to_rgb(hue: f32) -> vec3<f32> {
    let red = abs(hue * 6.0 - 3.0) - 1.0;
    let green = 2.0 - abs(hue * 6.0 - 2.0);
    let blue = 2.0 - abs(hue * 6.0 - 4.0);
    return clamp(vec3<f32>(red, green, blue), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let hue_rgb = hue_to_rgb(hsv.x);
    let chroma = hsv.z * hsv.y;
    return hue_rgb * chroma + hsv.z - chroma;
}

fn quad_uv(vertex_index: u32) -> vec2<f32> {
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
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var output: VertexOutput;
    let uv = quad_uv(vertex_index);
    output.uv = uv;
    output.effects_0 = transform.effects_0;
    output.effects_1 = transform.effects_1;
    if transform.visible < 0.5 {
        output.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return output;
    }

    let local_unflipped = vec2<f32>(
        uv.x * transform.logical_size.x - transform.rotation_center.x,
        transform.rotation_center.y - uv.y * transform.logical_size.y,
    ) * transform.scale;
    let local = vec2<f32>(local_unflipped.x * transform.flip_x, local_unflipped.y);
    // scratch direction 90° is the costume's unrotated, right-facing pose;
    // zero degrees points up. convert that compass-style direction into the
    // rotation from the costume's native orientation.
    let radians = (90.0 - transform.direction) * 0.017453292519943295;
    let cosine = cos(radians);
    let sine = sin(radians);
    let rotated = vec2<f32>(
        cosine * local.x - sine * local.y,
        sine * local.x + cosine * local.y,
    );
    let stage_position = transform.position + rotated;
    output.position = vec4<f32>(
        stage_position.x * (2.0 / 480.0),
        stage_position.y * (2.0 / 360.0),
        0.0,
        1.0,
    );
    return output;
}

fn shade_sprite(input: VertexOutput) -> vec4<f32> {
    let color_effect = input.effects_0.x;
    let fisheye_effect = input.effects_0.y;
    let whirl_effect = input.effects_0.z;
    let pixelate_effect = input.effects_0.w;
    let mosaic_effect = input.effects_1.x;
    let brightness_effect = input.effects_1.y;
    let ghost_effect = input.effects_1.z;
    if all(input.effects_0 == vec4<f32>(0.0)) && all(input.effects_1 == vec4<f32>(0.0)) {
        return textureSample(sprite_texture, sprite_sampler, input.uv);
    }

    // scratch's shader uses texture coordinates with +y upward. the costume
    // texture here uses the gpu's top-left origin, so convert around the sdfs.
    var scratch_uv = vec2<f32>(input.uv.x, 1.0 - input.uv.y);

    if mosaic_effect != 0.0 {
        let mosaic_scale = clamp(floor((abs(mosaic_effect) + 10.0) / 10.0 + 0.5), 1.0, 512.0);
        scratch_uv = fract(mosaic_scale * scratch_uv);
    }
    if pixelate_effect != 0.0 {
        let pixelate_scale = abs(pixelate_effect) / 10.0;
        let skin_size = vec2<f32>(textureDimensions(sprite_texture));
        let pixel_texel_size = skin_size / pixelate_scale;
        scratch_uv = (floor(scratch_uv * pixel_texel_size) + vec2<f32>(0.5)) / pixel_texel_size;
    }
    if whirl_effect != 0.0 {
        let center = vec2<f32>(0.5, 0.5);
        let offset = scratch_uv - center;
        let offset_magnitude = length(offset);
        let whirl_value = -whirl_effect * 0.017453292519943295;
        let whirl_factor = max(1.0 - offset_magnitude / 0.5, 0.0);
        let whirl_actual = whirl_value * whirl_factor * whirl_factor;
        let sine = sin(whirl_actual);
        let cosine = cos(whirl_actual);
        scratch_uv = vec2<f32>(
            cosine * offset.x + sine * offset.y,
            -sine * offset.x + cosine * offset.y,
        ) + center;
    }
    if fisheye_effect != 0.0 {
        let centered = (scratch_uv - vec2<f32>(0.5)) * 2.0;
        let vector_length = length(centered);
        if vector_length > EFFECT_EPSILON {
            let radius = pow(min(vector_length, 1.0), max(0.0, (fisheye_effect + 100.0) / 100.0))
                * max(1.0, vector_length);
            scratch_uv = vec2<f32>(0.5) + (centered / vector_length) * radius * 0.5;
        }
    }

    let sample_uv = vec2<f32>(scratch_uv.x, 1.0 - scratch_uv.y);
    let sampled = textureSample(sprite_texture, sprite_sampler, sample_uv);
    var color_srgb = linear_to_srgb(sampled.rgb);
    if color_effect != 0.0 || brightness_effect != 0.0 {
        if color_effect != 0.0 {
            var hsv = rgb_to_hsv(color_srgb);
            if hsv.z < 0.11 / 2.0 {
                hsv = vec3<f32>(0.0, 1.0, 0.11 / 2.0);
            } else if hsv.y < 0.09 {
                hsv = vec3<f32>(0.0, 0.09, hsv.z);
            }
            let hue_shift = color_effect / 200.0;
            hsv.x = fract(hsv.x + hue_shift);
            color_srgb = hsv_to_rgb(hsv);
        }
        if brightness_effect != 0.0 {
            let brightness = clamp(brightness_effect, -100.0, 100.0) / 100.0;
            color_srgb = clamp(color_srgb + vec3<f32>(brightness), vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }

    let ghost_alpha = 1.0 - clamp(ghost_effect, 0.0, 100.0) / 100.0;
    let alpha = sampled.a * ghost_alpha;
    return vec4<f32>(srgb_to_linear(color_srgb), alpha);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let straight = shade_sprite(input);
    return vec4<f32>(straight.rgb * straight.a, straight.a);
}

// the pen pass uses straight-alpha blending. each stamp carries the effects
// present when its scratch block ran, so this uses the same effect calculation
// as a normally rendered sprite.
@fragment
fn fs_stamp(input: VertexOutput) -> @location(0) vec4<f32> {
    return shade_sprite(input);
}
