use std::cmp::Ordering;

use super::ScratchValue;

// convert a scratch value to a number for arithmetic
fn to_number(value: &ScratchValue) -> f64 {
    let number = match value {
        ScratchValue::Number(number) => *number,

        ScratchValue::Boolean(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }

        ScratchValue::Text(text) => {
            let text = text.trim();

            if text.is_empty() {
                0.0
            } else {
                text.parse::<f64>().unwrap_or(0.0)
            }
        }
    };

    if number.is_nan() { 0.0 } else { number }
}

/// Convert a Scratch value to a number using Scratch's arithmetic rules.
pub fn scratch_to_number(value: impl Into<ScratchValue>) -> f64 {
    to_number(&value.into())
}

fn scratch_trig_round(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    if !value.is_finite() || value.abs() >= 1.0e21 {
        return value;
    }
    format!("{value:.10}").parse().unwrap_or(value)
}

/// Scratch's sine reporter: degrees in, rounded to 10 decimal places.
pub fn scratch_sin(value: impl Into<ScratchValue>) -> f64 {
    let degrees = scratch_to_number(value);
    scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).sin())
}

/// Scratch's cosine reporter: degrees in, rounded to 10 decimal places.
pub fn scratch_cos(value: impl Into<ScratchValue>) -> f64 {
    let degrees = scratch_to_number(value);
    scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).cos())
}

/// Scratch's tangent reporter: degrees in, rounded to 10 decimal places,
/// with Scratch VM's exact singularity handling.
pub fn scratch_tan(value: impl Into<ScratchValue>) -> f64 {
    let degrees = scratch_to_number(value) % 360.0;
    match degrees {
        -270.0 | 90.0 => f64::INFINITY,
        -90.0 | 270.0 => f64::NEG_INFINITY,
        _ => scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).tan()),
    }
}

/// Scratch's inverse sine in degrees, using a stable atan2 form.
pub fn scratch_asin_degrees(value: impl Into<ScratchValue>) -> f64 {
    let number = scratch_to_number(value);
    let radians = scratch_asin_radians(number);
    radians * 180.0 / std::f64::consts::PI
}

/// Scratch's inverse cosine in degrees, using the half-angle identity.
pub fn scratch_acos_degrees(value: impl Into<ScratchValue>) -> f64 {
    let number = scratch_to_number(value);
    let radians = 2.0 * scratch_asin_radians(((1.0 - number) / 2.0).sqrt());
    radians * 180.0 / std::f64::consts::PI
}

/// Scratch's inverse tangent in degrees.
pub fn scratch_atan_degrees(value: impl Into<ScratchValue>) -> f64 {
    let radians = scratch_to_number(value).atan();
    radians * 180.0 / std::f64::consts::PI
}

fn scratch_asin_radians(number: f64) -> f64 {
    number.atan2((1.0 - number * number).sqrt())
}

// convert a scratch value to text
fn to_text(value: &ScratchValue) -> String {
    match value {
        ScratchValue::Number(number) => {
            if number.is_nan() {
                "NaN".to_owned()
            } else if *number == f64::INFINITY {
                "Infinity".to_owned()
            } else if *number == f64::NEG_INFINITY {
                "-Infinity".to_owned()
            } else if *number == 0.0 {
                "0".to_owned()
            } else {
                number.to_string()
            }
        }

        ScratchValue::Text(text) => text.clone(),

        ScratchValue::Boolean(value) => value.to_string(),
    }
}

// convert a scratch value to a boolean
pub fn scratch_bool(value: impl Into<ScratchValue>) -> bool {
    match value.into() {
        ScratchValue::Boolean(value) => value,

        ScratchValue::Number(number) => number != 0.0 && !number.is_nan(),

        ScratchValue::Text(text) => {
            !text.is_empty() && text != "0" && !text.eq_ignore_ascii_case("false")
        }
    }
}

pub fn scratch_add(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> f64 {
    to_number(&left.into()) + to_number(&right.into())
}

pub fn scratch_subtract(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> f64 {
    to_number(&left.into()) - to_number(&right.into())
}

pub fn scratch_multiply(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> f64 {
    to_number(&left.into()) * to_number(&right.into())
}

pub fn scratch_divide(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> f64 {
    to_number(&left.into()) / to_number(&right.into())
}

// try a numeric conversion for comparisons
//
// whitespace is treated as text here, rather than as zero
fn comparison_number(value: &ScratchValue) -> Option<f64> {
    match value {
        ScratchValue::Number(number) => {
            if number.is_nan() {
                None
            } else {
                Some(*number)
            }
        }

        ScratchValue::Boolean(value) => Some(if *value { 1.0 } else { 0.0 }),

        ScratchValue::Text(text) => {
            let text = text.trim();

            if text.is_empty() {
                return None;
            }

            text.parse::<f64>().ok().filter(|number| !number.is_nan())
        }
    }
}

// compare numerically when both values can be read as numbers;
// otherwise compare their lowercase text
pub fn scratch_compare(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> Ordering {
    let left = left.into();
    let right = right.into();

    if let (Some(left_number), Some(right_number)) =
        (comparison_number(&left), comparison_number(&right))
    {
        return left_number
            .partial_cmp(&right_number)
            .unwrap_or(Ordering::Equal);
    }

    let left_text = to_text(&left).to_lowercase();
    let right_text = to_text(&right).to_lowercase();

    // javascript compares strings by utf-16 code units
    left_text.encode_utf16().cmp(right_text.encode_utf16())
}

pub fn scratch_equals(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> bool {
    scratch_compare(left, right) == Ordering::Equal
}

pub fn scratch_greater_than(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> bool {
    scratch_compare(left, right) == Ordering::Greater
}

pub fn scratch_less_than(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> bool {
    scratch_compare(left, right) == Ordering::Less
}

pub fn scratch_and(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> bool {
    scratch_bool(left) && scratch_bool(right)
}

pub fn scratch_or(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> bool {
    scratch_bool(left) || scratch_bool(right)
}

pub fn scratch_not(value: impl Into<ScratchValue>) -> bool {
    !scratch_bool(value)
}

pub fn scratch_join(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> String {
    let left = to_text(&left.into());
    let right = to_text(&right.into());

    format!("{left}{right}")
}

// scratch counts letters starting at 1
pub fn scratch_letter(index: impl Into<ScratchValue>, text: impl Into<ScratchValue>) -> String {
    let index = to_number(&index.into());
    let text = to_text(&text.into());

    if !index.is_finite() || index < 1.0 {
        return String::new();
    }

    let index = index.floor() as usize - 1;

    match text.encode_utf16().nth(index) {
        Some(code_unit) => String::from_utf16_lossy(&[code_unit]),
        None => String::new(),
    }
}

pub fn scratch_length(value: impl Into<ScratchValue>) -> f64 {
    let text = to_text(&value.into());
    text.encode_utf16().count() as f64
}

pub fn scratch_contains(text: impl Into<ScratchValue>, search: impl Into<ScratchValue>) -> bool {
    let text = to_text(&text.into()).to_lowercase();
    let search = to_text(&search.into()).to_lowercase();

    text.contains(&search)
}

// scratch modulo follows the sign of the divisor
pub fn scratch_mod(left: impl Into<ScratchValue>, right: impl Into<ScratchValue>) -> f64 {
    let left = to_number(&left.into());
    let right = to_number(&right.into());

    let remainder = left % right;

    if remainder / right < 0.0 {
        remainder + right
    } else {
        remainder
    }
}

// halfway values round toward positive infinity
pub fn scratch_round(value: impl Into<ScratchValue>) -> f64 {
    let number = to_number(&value.into());

    if !number.is_finite() || number == 0.0 {
        return number;
    }

    let lower = number.floor();

    let result = if number - lower >= 0.5 {
        lower + 1.0
    } else {
        lower
    };

    if result == 0.0 && number.is_sign_negative() {
        -0.0
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numbers_and_text() {
        assert!(scratch_equals("5", 5.0));
        assert!(scratch_equals("APPLE", "apple"));
        assert!(scratch_greater_than("banana", 0.0));
        assert!(!scratch_equals("", 0.0));
    }

    #[test]
    fn handles_text_operators() {
        assert_eq!(scratch_join("apple ", "banana"), "apple banana");
        assert_eq!(scratch_letter(1.0, "apple"), "a");
        assert_eq!(scratch_letter(10.0, "apple"), "");
        assert_eq!(scratch_length("Scratch"), 7.0);
        assert!(scratch_contains("Apple", "a"));
        assert!(scratch_contains(1234.0, 34.0));
    }

    #[test]
    fn handles_modulo_and_rounding() {
        assert_eq!(scratch_mod(4.0, 5.0), 4.0);
        assert_eq!(scratch_mod(-4.0, 5.0), 1.0);
        assert_eq!(scratch_round(1.5), 2.0);
        assert_eq!(scratch_round(-1.5), -1.0);
    }

    #[test]
    fn handles_boolean_conversion() {
        assert!(!scratch_bool("false"));
        assert!(!scratch_bool("0"));
        assert!(scratch_bool("hello"));
        assert!(scratch_not(false));
    }

    #[test]
    fn converts_scratch_values_to_numbers() {
        assert_eq!(scratch_to_number(5.5), 5.5);
        assert_eq!(scratch_to_number(" 5.5 "), 5.5);
        assert_eq!(scratch_to_number(true), 1.0);
        assert_eq!(scratch_to_number("not a number"), 0.0);
    }

    #[test]
    fn matches_scratch_trigonometry_precision_and_tangent_singularities() {
        assert_eq!(scratch_sin(30.0), 0.5);
        assert_eq!(scratch_cos(60.0), 0.5);
        assert_eq!(scratch_tan(45.0), 1.0);
        assert_eq!(scratch_tan(90.0), f64::INFINITY);
        assert_eq!(scratch_tan(-270.0), f64::INFINITY);
        assert_eq!(scratch_tan(-90.0), f64::NEG_INFINITY);
        assert_eq!(scratch_tan(270.0), f64::NEG_INFINITY);
    }
}
