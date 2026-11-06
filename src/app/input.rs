use std::sync::mpsc;
use winit::dpi::PhysicalPosition;

use crate::graphics::GraphicsState;
use crate::runtime::RuntimeInputEvent;

pub(super) fn forward_slider_edits(
    input_sender: &mpsc::Sender<RuntimeInputEvent>,
    graphics: &mut GraphicsState,
) {
    for event in graphics.read_changed_monitors() {
        let _ = input_sender.send(event);
    }
}

pub(super) fn scratch_key_name(key: &winit::keyboard::Key) -> Option<String> {
    use winit::keyboard::{Key, NamedKey};
    match key {
        Key::Character(value) => Some(value.to_string()),
        Key::Named(NamedKey::Space) => Some(" ".to_owned()),
        Key::Named(NamedKey::ArrowUp) => Some("ArrowUp".to_owned()),
        Key::Named(NamedKey::ArrowDown) => Some("ArrowDown".to_owned()),
        Key::Named(NamedKey::ArrowLeft) => Some("ArrowLeft".to_owned()),
        Key::Named(NamedKey::ArrowRight) => Some("ArrowRight".to_owned()),
        Key::Named(NamedKey::Enter) => Some("Enter".to_owned()),
        _ => None,
    }
}

pub(super) fn position_distance(a: PhysicalPosition<f64>, b: PhysicalPosition<f64>) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
