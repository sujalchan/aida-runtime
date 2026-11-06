pub(crate) fn linear_rgba(color: [u8; 4]) -> [f32; 4] {
    let linear = |channel: u8| {
        let srgb = channel as f32 / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        linear(color[0]),
        linear(color[1]),
        linear(color[2]),
        color[3] as f32 / 255.0,
    ]
}
