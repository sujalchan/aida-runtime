//! Scratch values and borrowed conversions shared by generated code and runtime operators.

use super::operators::{scratch_string, scratch_to_number};

#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    /// undefined represents results from legacy Scratch VM reporter no-ops.
    Undefined,
}

impl ScratchValue {
    /// A value may use a numeric local only when its comparison fallback text
    /// is identical to the text of that number. Other dynamic values fall back.
    #[inline]
    pub fn number_or_nan(&self) -> f64 {
        match self {
            Self::Number(value) => *value,
            _ => f64::NAN,
        }
    }

    pub fn canonical_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) if !value.is_nan() => Some(*value),
            Self::Text(text) => match text.as_str() {
                "0" => Some(0.0),
                "1" => Some(1.0),
                "-1" => Some(-1.0),
                _ => {
                    let value = text.parse::<f64>().ok()?;
                    (!value.is_nan() && scratch_string(value) == *text).then_some(value)
                }
            },
            _ => None,
        }
    }
}

/// a borrowed view of a scratch value accepted by runtime operators.
#[derive(Debug, Clone, Copy)]
pub enum ScratchValueView<'a> {
    Number(f64),
    Text(&'a str),
    Boolean(bool),
    Undefined,
}

/// A Scratch list selector converted once for repeated synchronous reads.
/// Special selectors remain text so "last" and random selection retain their
/// original list-specific behavior.
#[derive(Clone, Copy, Debug)]
pub struct CachedListIndex(ScratchValueView<'static>);

impl CachedListIndex {
    #[inline]
    pub fn is_numeric(self) -> bool {
        matches!(self.0, ScratchValueView::Number(_))
    }

    pub fn new(value: &ScratchValue) -> Self {
        match value {
            ScratchValue::Text(text) if text == "last" => Self(ScratchValueView::Text("last")),
            ScratchValue::Text(text) if text == "random" => Self(ScratchValueView::Text("random")),
            ScratchValue::Text(text) if text == "any" => Self(ScratchValueView::Text("any")),
            ScratchValue::Number(number) => Self(ScratchValueView::Number(*number)),
            _ => Self(ScratchValueView::Number(scratch_to_number(value))),
        }
    }
}

impl ScratchInput for CachedListIndex {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        self.0
    }
}

/// a value that a scratch operator can inspect without taking ownership.
pub trait ScratchInput {
    fn scratch_view(&self) -> ScratchValueView<'_>;
}

impl ScratchInput for ScratchValue {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        match self {
            Self::Number(number) => ScratchValueView::Number(*number),
            Self::Text(text) => ScratchValueView::Text(text),
            Self::Boolean(value) => ScratchValueView::Boolean(*value),
            Self::Undefined => ScratchValueView::Undefined,
        }
    }
}

impl<T: ScratchInput + ?Sized> ScratchInput for &T {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        T::scratch_view(self)
    }
}

impl ScratchInput for f64 {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Number(*self)
    }
}

impl ScratchInput for bool {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Boolean(*self)
    }
}

impl ScratchInput for str {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Text(self)
    }
}

impl ScratchInput for String {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Text(self)
    }
}

impl From<f64> for ScratchValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<bool> for ScratchValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<&str> for ScratchValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for ScratchValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
