use super::{PenColorParameter, ScratchInput, ScratchValueView, scratch_to_number};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenColorState {
    pub hue: f64,
    pub saturation: f64,
    pub brightness: f64,
    pub transparency: f64,
    pub legacy_shade: f64,
    pub rgba: [f32; 4],
}

impl Default for PenColorState {
    fn default() -> Self {
        let mut color = Self {
            hue: 66.66,
            saturation: 100.0,
            brightness: 100.0,
            transparency: 0.0,
            legacy_shade: 50.0,
            rgba: [0.0, 0.0, 1.0, 1.0],
        };
        color.update_rgba();
        color
    }
}

impl PenColorState {
    pub fn set_rgb_color(&mut self, input: impl ScratchInput) {
        let (rgb, alpha) = parse_scratch_color(input.scratch_view());
        let (hue, saturation, brightness) =
            rgb_to_hsv(rgb.map(|component| component as f64 / 255.0));
        self.hue = hue * 100.0 / 360.0;
        self.saturation = saturation * 100.0;
        self.brightness = brightness * 100.0;
        self.transparency = 100.0 * (1.0 - alpha as f64 / 255.0);
        self.legacy_shade = self.brightness / 2.0;
        self.update_rgba();
    }

    pub fn set_or_change_parameter(
        &mut self,
        parameter: impl Into<PenColorParameter>,
        value: f64,
        change: bool,
    ) {
        let value = scratch_to_number(value);
        match parameter.into() {
            PenColorParameter::Color => {
                self.hue = scratch_wrap_color(value + if change { self.hue } else { 0.0 });
            }
            PenColorParameter::Saturation => {
                self.saturation =
                    (value + if change { self.saturation } else { 0.0 }).clamp(0.0, 100.0);
            }
            PenColorParameter::Brightness => {
                self.brightness =
                    (value + if change { self.brightness } else { 0.0 }).clamp(0.0, 100.0);
            }
            PenColorParameter::Transparency => {
                self.transparency =
                    (value + if change { self.transparency } else { 0.0 }).clamp(0.0, 100.0);
            }
            PenColorParameter::Unsupported => return,
        }
        self.update_rgba();
    }

    pub fn set_legacy_hue(&mut self, hue: f64) {
        self.hue = scratch_wrap_color(scratch_to_number(hue) / 2.0);
        self.transparency = 0.0;
        self.update_legacy_shade_color();
    }

    pub fn change_legacy_hue(&mut self, change: f64) {
        self.hue = scratch_wrap_color(self.hue + scratch_to_number(change) / 2.0);
        self.update_legacy_shade_color();
    }

    pub fn set_legacy_shade(&mut self, shade: f64) {
        let shade = scratch_to_number(shade) % 200.0;
        self.legacy_shade = if shade < 0.0 { shade + 200.0 } else { shade };
        self.update_legacy_shade_color();
    }

    pub fn change_legacy_shade(&mut self, change: f64) {
        self.set_legacy_shade(self.legacy_shade + scratch_to_number(change));
    }

    fn update_legacy_shade_color(&mut self) {
        let pure_hue = hsv_to_rgb(self.hue * 3.6, 1.0, 1.0);
        let shade = if self.legacy_shade > 100.0 {
            200.0 - self.legacy_shade
        } else {
            self.legacy_shade
        };
        let rgb = if shade < 50.0 {
            mix_rgb([0.0; 3], pure_hue, (10.0 + shade) / 60.0)
        } else {
            mix_rgb(pure_hue, [1.0; 3], (shade - 50.0) / 60.0)
        };
        let (hue, saturation, brightness) = rgb_to_hsv(rgb);
        self.hue = hue * 100.0 / 360.0;
        self.saturation = saturation * 100.0;
        self.brightness = brightness * 100.0;
        self.update_rgba();
    }

    fn update_rgba(&mut self) {
        let rgb = hsv_to_rgb(
            self.hue * 3.6,
            self.saturation / 100.0,
            self.brightness / 100.0,
        );
        self.rgba = [
            rgb[0] as f32,
            rgb[1] as f32,
            rgb[2] as f32,
            (1.0 - self.transparency / 100.0) as f32,
        ];
    }
}

fn scratch_wrap_color(value: f64) -> f64 {
    // Scratch's MathUtil.wrapClamp uses an inclusive 0..=100 interval.
    value - ((value / 101.0).floor() * 101.0)
}

fn parse_scratch_color(value: ScratchValueView<'_>) -> ([u8; 3], u8) {
    if let ScratchValueView::Text(text) = value
        && text.starts_with('#')
    {
        let hex = &text[1..];
        let expanded = if hex.len() == 3 {
            let mut expanded = String::with_capacity(6);
            for byte in hex.bytes() {
                expanded.push(byte as char);
                expanded.push(byte as char);
            }
            expanded
        } else {
            hex.to_owned()
        };
        if expanded.len() == 6 && expanded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let channel = |offset| u8::from_str_radix(&expanded[offset..offset + 2], 16).unwrap();
            return ([channel(0), channel(2), channel(4)], 255);
        }
        return ([0, 0, 0], 255);
    }

    let number = match value {
        ScratchValueView::Number(number) => number,
        ScratchValueView::Boolean(value) => f64::from(value),
        ScratchValueView::Text(text) => scratch_to_number(text),
    };
    let wrapped = if number.is_finite() {
        number.trunc().rem_euclid(4_294_967_296.0) as u32
    } else {
        0
    };
    let alpha = (wrapped >> 24) as u8;
    let alpha = if alpha == 0 { 255 } else { alpha };
    (
        [
            ((wrapped >> 16) & 255) as u8,
            ((wrapped >> 8) & 255) as u8,
            (wrapped & 255) as u8,
        ],
        alpha,
    )
}

fn rgb_to_hsv([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let min = r.min(g).min(b);
    let max = r.max(g).max(b);
    let mut hue = 0.0;
    let mut saturation = 0.0;
    if min != max {
        let (difference, sector) = if r == min {
            (g - b, 3.0)
        } else if g == min {
            (b - r, 5.0)
        } else {
            (r - g, 1.0)
        };
        hue = (sector - difference / (max - min)) * 60.0;
        if hue == 360.0 {
            hue = 0.0;
        }
        saturation = (max - min) / max;
    }
    (hue, saturation, max)
}

fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> [f64; 3] {
    let hue = hue.rem_euclid(360.0);
    let saturation = saturation.clamp(0.0, 1.0);
    let value = value.clamp(0.0, 1.0);
    let sector = (hue / 60.0).floor();
    let fraction = hue / 60.0 - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    let rgb = match sector as u8 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    };
    rgb.map(|component| (component * 255.0).floor() / 255.0)
}

fn mix_rgb(first: [f64; 3], second: [f64; 3], fraction: f64) -> [f64; 3] {
    std::array::from_fn(|index| first[index] * (1.0 - fraction) + second[index] * fraction)
}

#[cfg(test)]
mod tests {
    use super::{PenColorParameter, PenColorState, scratch_to_number};

    #[test]
    fn color_parameter_names_map_to_compact_runtime_values() {
        assert_eq!(PenColorParameter::from("color"), PenColorParameter::Color);
        assert_eq!(
            PenColorParameter::from("saturation"),
            PenColorParameter::Saturation
        );
        assert_eq!(
            PenColorParameter::from("brightness"),
            PenColorParameter::Brightness
        );
        assert_eq!(
            PenColorParameter::from("transparency"),
            PenColorParameter::Transparency
        );
        assert_eq!(
            PenColorParameter::from("unknown"),
            PenColorParameter::Unsupported
        );
        assert_eq!(
            PenColorParameter::from(String::from("color")),
            PenColorParameter::Color
        );
    }

    #[test]
    fn hue_uses_scratch_inclusive_wrap_and_other_parameters_clamp() {
        let mut state = PenColorState::default();
        state.set_or_change_parameter("color", 100.0, false);
        assert_eq!(state.hue, 100.0);
        assert_eq!(state.rgba, [1.0, 0.0, 0.0, 1.0]);
        state.set_or_change_parameter("color", 101.0, false);
        assert_eq!(state.hue, 0.0);
        state.set_or_change_parameter("saturation", 150.0, false);
        state.set_or_change_parameter("brightness", -10.0, false);
        state.set_or_change_parameter("transparency", 150.0, false);
        assert_eq!(
            (state.saturation, state.brightness, state.transparency),
            (100.0, 0.0, 100.0)
        );
        assert_eq!(state.rgba[3], 0.0);
    }

    // lock down scratch's inclusive hue endpoints and negative wrap behavior.
    #[test]
    fn hue_wraps_over_scratchs_inclusive_zero_to_one_hundred_range() {
        let mut state = PenColorState::default();
        state.set_or_change_parameter("color", -1.0, false);
        assert_eq!(state.hue, 100.0);
        state.set_or_change_parameter("color", 101.0, false);
        assert_eq!(state.hue, 0.0);
        state.set_or_change_parameter("color", 100.0, false);
        state.set_or_change_parameter("color", 1.0, true);
        assert_eq!(state.hue, 0.0);
    }

    // check the three non-hue controls clamp both set and accumulated values.
    #[test]
    fn color_parameters_clamp_each_set_or_change_result() {
        let mut state = PenColorState::default();
        state.set_or_change_parameter("saturation", -1.0, false);
        state.set_or_change_parameter("saturation", 125.0, true);
        state.set_or_change_parameter("brightness", 101.0, false);
        state.set_or_change_parameter("brightness", -125.0, true);
        state.set_or_change_parameter("transparency", 40.5, false);
        state.set_or_change_parameter("transparency", 75.0, true);
        assert_eq!(
            (state.saturation, state.brightness, state.transparency),
            (100.0, 0.0, 100.0)
        );
        assert_eq!(state.rgba[3], 0.0);
    }

    // confirm invalid reporter text uses scratch's zero numeric conversion.
    #[test]
    fn invalid_numeric_input_uses_scratch_zero_conversion_before_clamping() {
        let mut state = PenColorState::default();
        let value = scratch_to_number("not a number");
        state.set_or_change_parameter("brightness", value, false);
        assert_eq!(state.brightness, 0.0);
        assert_eq!(state.rgba[..3], [0.0, 0.0, 0.0]);
    }

    // verify all four controls update hsv and alpha using scratch's channel rounding.
    #[test]
    fn color_parameter_values_follow_scratch_hsv_and_transparency_mapping() {
        let mut state = PenColorState::default();
        for parameter in ["color", "saturation", "brightness", "transparency"] {
            state.set_or_change_parameter(parameter, 10.0, true);
        }
        assert!((state.hue - 76.66).abs() < 0.00001);
        assert_eq!(
            (state.saturation, state.brightness, state.transparency),
            (100.0, 100.0, 10.0)
        );
        for parameter in ["color", "saturation", "brightness", "transparency"] {
            state.set_or_change_parameter(parameter, 50.0, false);
        }
        assert_eq!(
            (
                state.hue,
                state.saturation,
                state.brightness,
                state.transparency
            ),
            (50.0, 50.0, 50.0, 50.0)
        );
        let expected = [63.0 / 255.0, 127.0 / 255.0, 127.0 / 255.0, 0.5];
        for (actual, expected) in state.rgba.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.00001);
        }
    }

    #[test]
    fn rgb_color_resets_alpha_for_hex_and_reads_numeric_alpha_byte() {
        let mut state = PenColorState::default();
        state.set_rgb_color("#00ff00");
        assert_eq!(state.rgba, [0.0, 1.0, 0.0, 1.0]);
        state.set_rgb_color("#0f0");
        assert_eq!(state.rgba, [0.0, 1.0, 0.0, 1.0]);
        state.set_rgb_color("65280");
        assert_eq!(state.rgba, [0.0, 1.0, 0.0, 1.0]);
        state.set_rgb_color(65_280.0);
        assert_eq!(state.rgba, [0.0, 1.0, 0.0, 1.0]);
        state.set_rgb_color(0x8000_ff00_u32 as f64);
        assert!((state.rgba[3] - 128.0 / 255.0).abs() < 0.00001);
        state.set_rgb_color("#bad-color");
        assert_eq!(state.rgba, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn legacy_shade_wraps_by_two_hundred_and_keeps_its_own_value() {
        let mut state = PenColorState::default();
        state.set_legacy_shade(-10.0);
        assert_eq!(state.legacy_shade, 190.0);
        state.change_legacy_shade(20.0);
        assert_eq!(state.legacy_shade, 10.0);
    }
}
