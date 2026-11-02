#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
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
