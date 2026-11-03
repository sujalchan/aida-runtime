use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    sync::{Arc, RwLock, mpsc},
    time::{Duration, Instant},
};

use aida_ast::parser::ScratchProject;
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{WindowAttributes, WindowId},
};

use crate::{
    graphics::{GraphicsState, RenderOutcome},
    runtime::{ObservableEvent, RuntimeInputEvent, RuntimeSnapshot, ScratchValue},
};

pub(crate) enum RendererEvent {
    SnapshotUpdated,
    ExecutionFinished,
}

pub(crate) struct RendererApp {
    graphics: Option<GraphicsState>,
    project: Arc<ScratchProject>,
    assets_dir: PathBuf,
    window_title: String,
    latest: Arc<RwLock<Arc<RuntimeSnapshot>>>,
    notification_pending: Arc<AtomicBool>,
    observable_events: Option<mpsc::Receiver<ObservableEvent>>,
    stop_sender: mpsc::Sender<()>,
    acknowledgement_sender: mpsc::Sender<usize>,
    input_sender: mpsc::Sender<RuntimeInputEvent>,
    expected_variables: Vec<(String, ScratchValue)>,
    dev_mode: bool,
    test_mode: bool,
    hd_pen_mode: bool,
    cursor_position: Option<PhysicalPosition<f64>>,
    dragging_variable: bool,
    dragging_slider: bool,
    last_variable_click: Option<(Instant, PhysicalPosition<f64>)>,
    error: Option<String>,
    redraw_pending: bool,
    pending_snapshot_tick: usize,
}

impl RendererApp {
    pub(crate) fn new(
        project: Arc<ScratchProject>,
        assets_dir: PathBuf,
        window_title: String,
        latest: Arc<RwLock<Arc<RuntimeSnapshot>>>,
        notification_pending: Arc<AtomicBool>,
        observable_events: Option<mpsc::Receiver<ObservableEvent>>,
        stop_sender: mpsc::Sender<()>,
        acknowledgement_sender: mpsc::Sender<usize>,
        input_sender: mpsc::Sender<RuntimeInputEvent>,
        expected_variables: Vec<(String, ScratchValue)>,
        dev_mode: bool,
        test_mode: bool,
        hd_pen_mode: bool,
    ) -> Self {
        Self {
            graphics: None,
            project,
            assets_dir,
            window_title,
            latest,
            notification_pending,
            observable_events,
            stop_sender,
            acknowledgement_sender,
            input_sender,
            expected_variables,
            dev_mode,
            test_mode,
            hd_pen_mode,
            cursor_position: None,
            dragging_variable: false,
            dragging_slider: false,
            last_variable_click: None,
            error: None,
            redraw_pending: false,
            pending_snapshot_tick: 0,
        }
    }

    fn snapshot(&self) -> Arc<RuntimeSnapshot> {
        Arc::clone(
            &self
                .latest
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn sync_latest_snapshot(&mut self) {
        let snapshot = self.snapshot();
        self.pending_snapshot_tick = snapshot.tick;
        if let Some(graphics) = self.graphics.as_mut() {
            match graphics.sync_runtime_snapshot(&snapshot) {
                Ok(true) => {
                    graphics.window().request_redraw();
                    self.redraw_pending = true;
                }
                Ok(false) => {
                    let _ = self.acknowledgement_sender.send(self.pending_snapshot_tick);
                }
                Err(error) => {
                    self.error = Some(format!("failed to update Scratch renderer state: {error}"));
                }
            }
        }
    }

    fn flush_observable_events(&mut self) {
        if let Some(receiver) = &self.observable_events {
            for event in receiver.try_iter() {
                match event {
                    ObservableEvent::Output { label, value } => {
                        println!("observable output {label}: {value}");
                    }
                    ObservableEvent::VariableChange {
                        key,
                        old_value,
                        new_value,
                    } => println!("variable change {key}: {old_value} -> {new_value}"),
                }
            }
        }
    }

    fn redraw(&mut self) {
        self.redraw_pending = false;
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };
        if self.dev_mode
            && let Err(error) = graphics.update_variable_shadow(Instant::now())
        {
            self.error = Some(format!("failed to update variable monitor shadow: {error}"));
        }
        let keep_animating =
            self.dev_mode && (self.dragging_variable || graphics.variable_shadow_is_animating());
        let tick = self.pending_snapshot_tick;
        let acknowledgement_sender = &self.acknowledgement_sender;
        match graphics.render_with_submission_ack(|| {
            let _ = acknowledgement_sender.send(tick);
        }) {
            RenderOutcome::Presented => {}
            RenderOutcome::SurfaceRestored => {
                graphics.window().request_redraw();
                self.redraw_pending = true;
            }
            RenderOutcome::Unavailable => {}
        }
        if keep_animating {
            graphics.window().request_redraw();
            self.redraw_pending = true;
        }
    }

    fn request_redraw(&mut self) {
        if let Some(graphics) = self.graphics.as_ref() {
            graphics.window().request_redraw();
            self.redraw_pending = true;
        }
    }

    fn stage_pointer(&self) -> Option<(f32, f32)> {
        self.cursor_position.and_then(|position| {
            self.graphics
                .as_ref()
                .and_then(|graphics| graphics.window_to_stage(position))
        })
    }

    fn verify_expected_variables(&self) -> Result<(), String> {
        let snapshot = self.snapshot();
        for (key, expected) in &self.expected_variables {
            let actual = snapshot
                .variables
                .iter()
                .find(|value| &value.key == key)
                .map(|value| &value.value)
                .ok_or_else(|| format!("expected variable key `{key}` was not observed"))?;
            if actual != expected {
                return Err(format!(
                    "variable `{key}`: expected {expected:?}, observed {actual:?}"
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}

fn forward_slider_edits(
    input_sender: &mpsc::Sender<RuntimeInputEvent>,
    graphics: &mut GraphicsState,
) {
    for event in graphics.read_changed_monitors() {
        let _ = input_sender.send(event);
    }
}

impl ApplicationHandler<RendererEvent> for RendererApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            self.sync_latest_snapshot();
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(&self.window_title)
            .with_inner_size(PhysicalSize::new(960, 720))
            .with_min_inner_size(PhysicalSize::new(480, 360));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.error = Some(format!("failed to create graphical window: {error}"));
                let _ = self.stop_sender.send(());
                event_loop.exit();
                return;
            }
        };
        let snapshot = self.snapshot();
        match pollster::block_on(GraphicsState::new(
            window,
            false,
            self.dev_mode,
            Arc::clone(&self.project),
            (*snapshot).clone(),
            &self.assets_dir,
        )) {
            Ok(mut graphics) => {
                graphics.set_hd_pen_mode(self.hd_pen_mode);
                self.graphics = Some(graphics);
                self.sync_latest_snapshot();
            }
            Err(error) => {
                self.error = Some(format!("graphics initialization failed: {error}"));
                let _ = self.stop_sender.send(());
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                let _ = self.stop_sender.send(());
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.resize(size);
                }
                self.sync_latest_snapshot();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.dev_mode
                    && event.state == ElementState::Pressed
                    && self
                        .graphics
                        .as_ref()
                        .is_some_and(GraphicsState::context_menu_is_open)
                {
                    use crate::graphics::SliderRangeEditorInput as Input;
                    let input = match &event.logical_key {
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                            Some(Input::Apply)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                            Some(Input::Cancel)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                            Some(Input::Backspace)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                            Some(Input::SwitchField)
                        }
                        _ => event.text.as_deref().map(Input::Text),
                    };
                    if let (Some(input), Some(graphics)) = (input, self.graphics.as_mut()) {
                        match graphics.edit_variable_slider_range(input) {
                            Ok(true) => self.request_redraw(),
                            Ok(false) => {}
                            Err(error) => {
                                self.error =
                                    Some(format!("failed to edit variable slider range: {error}"));
                            }
                        }
                    }
                    return;
                }
                if let Some(name) = scratch_key_name(&event.logical_key) {
                    let _ = self.input_sender.send(RuntimeInputEvent::Key {
                        name,
                        pressed: event.state == ElementState::Pressed,
                    });
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(position);
                let stage_pointer = self.stage_pointer();
                if let Some((x, y)) = self
                    .graphics
                    .as_ref()
                    .and_then(|graphics| graphics.window_to_stage_unclamped(position))
                {
                    let _ = self.input_sender.send(RuntimeInputEvent::MousePosition {
                        x: x as f64,
                        y: y as f64,
                    });
                }

                let menu_open = self.dev_mode
                    && self
                        .graphics
                        .as_ref()
                        .is_some_and(GraphicsState::context_menu_is_open);
                if menu_open {
                    if let Some(graphics) = self.graphics.as_mut() {
                        match graphics.update_variable_context_menu_hover(stage_pointer) {
                            Ok(true) => self.request_redraw(),
                            Ok(false) => {}
                            Err(error) => {
                                self.error = Some(format!(
                                    "failed to update variable context menu: {error}"
                                ));
                            }
                        }
                    }
                } else if !self.dragging_variable && !self.dragging_slider {
                    if let Some((x, y)) = stage_pointer {
                        if let Some(graphics) = self.graphics.as_mut() {
                            match graphics.update_variable_slider_hover(Some((x, y))) {
                                Ok(true) => self.request_redraw(),
                                Ok(false) => {}
                                Err(error) => {
                                    self.error = Some(format!(
                                        "failed to update variable slider hover: {error}"
                                    ));
                                }
                            }
                        }
                    }
                }

                if self.dragging_variable || self.dragging_slider {
                    if let (Some((x, y)), Some(graphics)) = (stage_pointer, self.graphics.as_mut())
                    {
                        match graphics.drag_variable_to(x, y) {
                            Ok(_) => {
                                forward_slider_edits(&self.input_sender, graphics);
                                self.request_redraw();
                            }
                            Err(error) => {
                                self.error =
                                    Some(format!("failed to move variable monitor: {error}"));
                            }
                        }
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_position = None;
                if self.dev_mode {
                    if let Some(graphics) = self.graphics.as_mut() {
                        let changed = if graphics.context_menu_is_open() {
                            graphics.update_variable_context_menu_hover(None)
                        } else {
                            graphics.update_variable_slider_hover(None)
                        };
                        match changed {
                            Ok(true) => self.request_redraw(),
                            Ok(false) => {}
                            Err(error) => {
                                self.error = Some(format!(
                                    "failed to clear variable monitor hover: {error}"
                                ));
                            }
                        }
                    }
                } else if let Some(graphics) = self.graphics.as_mut() {
                    match graphics.update_variable_slider_hover(None) {
                        Ok(true) => self.request_redraw(),
                        Ok(false) => {}
                        Err(error) => {
                            self.error =
                                Some(format!("failed to clear variable slider hover: {error}"));
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let stage_pointer = self.stage_pointer();
                    let _ = self.input_sender.send(RuntimeInputEvent::MouseButton {
                        pressed: state == ElementState::Pressed,
                    });

                    match state {
                        ElementState::Pressed => {
                            let menu_open = self.dev_mode
                                && self
                                    .graphics
                                    .as_ref()
                                    .is_some_and(GraphicsState::context_menu_is_open);
                            if menu_open {
                                if let Some(graphics) = self.graphics.as_mut() {
                                    match graphics.activate_variable_context_menu(stage_pointer) {
                                        Ok(true) => self.request_redraw(),
                                        Ok(false) => {}
                                        Err(error) => {
                                            self.error = Some(format!(
                                                "failed to activate variable menu: {error}"
                                            ));
                                        }
                                    }
                                }
                                return;
                            }

                            if let (Some((x, y)), Some(graphics)) =
                                (stage_pointer, self.graphics.as_mut())
                            {
                                let now = Instant::now();
                                let is_double_click = self.cursor_position.is_some_and(|pos| {
                                    self.last_variable_click.is_some_and(
                                        |(last_time, last_position)| {
                                            now.duration_since(last_time)
                                                <= Duration::from_millis(500)
                                                && position_distance(last_position, pos) <= 8.0
                                        },
                                    )
                                });
                                if self.dev_mode && is_double_click {
                                    self.last_variable_click = None;
                                    match graphics.toggle_variable_compact(x, y) {
                                        Ok(true) => {
                                            self.request_redraw();
                                            return;
                                        }
                                        Ok(false) => {}
                                        Err(error) => {
                                            self.error = Some(format!(
                                                "failed to toggle monitor display: {error}"
                                            ));
                                        }
                                    }
                                }

                                let hit_monitor = graphics.variable_monitor_at(x, y);
                                self.last_variable_click = if hit_monitor {
                                    self.cursor_position.map(|position| (now, position))
                                } else {
                                    None
                                };

                                match graphics.begin_variable_slider(x, y) {
                                    Ok(true) => {
                                        self.dragging_slider = true;
                                        forward_slider_edits(&self.input_sender, graphics);
                                        self.request_redraw();
                                        return;
                                    }
                                    Ok(false) => {}
                                    Err(error) => {
                                        self.error = Some(format!(
                                            "failed to start monitor slider: {error}"
                                        ));
                                    }
                                }
                                if self.dev_mode {
                                    match graphics.begin_variable_drag(x, y) {
                                        Ok(true) => {
                                            self.dragging_variable = true;
                                            self.request_redraw();
                                        }
                                        Ok(false) => self.last_variable_click = None,
                                        Err(error) => {
                                            self.error = Some(format!(
                                                "failed to start variable drag: {error}"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        ElementState::Released => {
                            if self.dragging_variable || self.dragging_slider {
                                if let Some(graphics) = self.graphics.as_mut() {
                                    if let Some((x, y)) = stage_pointer {
                                        match graphics.drag_variable_to(x, y) {
                                            Ok(_) => {
                                                forward_slider_edits(&self.input_sender, graphics)
                                            }
                                            Err(error) => {
                                                self.error = Some(format!(
                                                    "failed to finish variable drag: {error}"
                                                ));
                                            }
                                        }
                                    }
                                    if let Err(error) = graphics.end_variable_drag() {
                                        self.error =
                                            Some(format!("failed to end variable drag: {error}"));
                                    }
                                    if let Err(error) =
                                        graphics.update_variable_slider_hover(stage_pointer)
                                    {
                                        self.error =
                                            Some(format!("failed to update slider hover: {error}"));
                                    }
                                }
                                self.dragging_variable = false;
                                self.dragging_slider = false;
                                self.request_redraw();
                            }
                        }
                    }
                } else if button == MouseButton::Right
                    && state == ElementState::Pressed
                    && self.dev_mode
                {
                    if self.dragging_variable || self.dragging_slider {
                        return;
                    }
                    let stage_pointer = self.stage_pointer();
                    if let Some(graphics) = self.graphics.as_mut() {
                        match graphics.open_variable_context_menu(stage_pointer) {
                            Ok(true) => self.request_redraw(),
                            Ok(false) => {}
                            Err(error) => {
                                self.error =
                                    Some(format!("failed to open variable context menu: {error}"));
                            }
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RendererEvent) {
        match event {
            RendererEvent::SnapshotUpdated => {
                self.notification_pending
                    .store(false, std::sync::atomic::Ordering::Release);
                self.sync_latest_snapshot();
                self.flush_observable_events();
            }
            RendererEvent::ExecutionFinished => {
                self.sync_latest_snapshot();
                self.flush_observable_events();
                if let Err(error) = self.verify_expected_variables() {
                    self.error = Some(format!("assertion failed: {error}"));
                }
                if self.test_mode {
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn scratch_key_name(key: &winit::keyboard::Key) -> Option<String> {
    use winit::keyboard::{Key, NamedKey};
    match key {
        Key::Character(value) => Some(value.to_lowercase()),
        Key::Named(NamedKey::Space) => Some("space".to_owned()),
        Key::Named(NamedKey::ArrowUp) => Some("up arrow".to_owned()),
        Key::Named(NamedKey::ArrowDown) => Some("down arrow".to_owned()),
        Key::Named(NamedKey::ArrowLeft) => Some("left arrow".to_owned()),
        Key::Named(NamedKey::ArrowRight) => Some("right arrow".to_owned()),
        Key::Named(NamedKey::Enter) => Some("enter".to_owned()),
        Key::Named(NamedKey::Escape) => Some("escape".to_owned()),
        _ => None,
    }
}

fn position_distance(a: PhysicalPosition<f64>, b: PhysicalPosition<f64>) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::scratch_key_name;
    use winit::keyboard::{Key, NamedKey};

    #[test]
    fn translates_winit_keyboard_keys_to_scratch_names() {
        assert_eq!(
            scratch_key_name(&Key::Character("A".into())),
            Some("a".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::Space)),
            Some("space".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::ArrowLeft)),
            Some("left arrow".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::Enter)),
            Some("enter".into())
        );
    }
}
