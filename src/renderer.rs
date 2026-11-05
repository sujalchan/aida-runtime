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
    graphics::{GraphicsState, ListEditorInput, ListMonitorFileAction, RenderOutcome},
    runtime::{ObservableEvent, PenCommand, RuntimeInputEvent, RuntimeSnapshot, ScratchValue},
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
    pending_pen_commands: Arc<RwLock<Vec<PenCommand>>>,
    notification_pending: Arc<AtomicBool>,
    observable_events: Option<mpsc::Receiver<ObservableEvent>>,
    stop_sender: mpsc::Sender<()>,
    execution_start_sender: Option<mpsc::Sender<()>>,
    acknowledgement_sender: mpsc::Sender<usize>,
    input_sender: mpsc::Sender<RuntimeInputEvent>,
    requested_frame_interval: Duration,
    expected_variables: Vec<(String, ScratchValue)>,
    dev_mode: bool,
    test_mode: bool,
    hd_pen_mode: bool,
    cursor_position: Option<PhysicalPosition<f64>>,
    pointer_down_at: Option<PhysicalPosition<f64>>,
    pointer_was_dragged: bool,
    pointer_suppressed: bool,
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
        pending_pen_commands: Arc<RwLock<Vec<PenCommand>>>,
        notification_pending: Arc<AtomicBool>,
        observable_events: Option<mpsc::Receiver<ObservableEvent>>,
        stop_sender: mpsc::Sender<()>,
        execution_start_sender: mpsc::Sender<()>,
        acknowledgement_sender: mpsc::Sender<usize>,
        input_sender: mpsc::Sender<RuntimeInputEvent>,
        requested_frame_interval: Duration,
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
            pending_pen_commands,
            notification_pending,
            observable_events,
            stop_sender,
            execution_start_sender: Some(execution_start_sender),
            acknowledgement_sender,
            input_sender,
            requested_frame_interval,
            expected_variables,
            dev_mode,
            test_mode,
            hd_pen_mode,
            cursor_position: None,
            pointer_down_at: None,
            pointer_was_dragged: false,
            pointer_suppressed: false,
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
        let pen_commands = {
            let mut pending = self
                .pending_pen_commands
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *pending)
        };
        self.pending_snapshot_tick = snapshot.tick;
        if let Some(graphics) = self.graphics.as_mut() {
            match graphics.sync_runtime_snapshot(&snapshot, pen_commands) {
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
        match graphics.render_with_present_ack(|| {
            let _ = acknowledgement_sender.send(tick);
        }) {
            RenderOutcome::Presented => {
                if let Some(sender) = self.execution_start_sender.take() {
                    let _ = sender.send(());
                }
            }
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

    // send one resolved pointer edge; only the presentation thread performs sprite picking.
    fn send_pointer_button(&self, pressed: bool, monitor_interaction: bool) {
        let stage_pointer = self.stage_pointer();
        let picked = if monitor_interaction {
            None
        } else {
            stage_pointer.and_then(|point| {
                self.graphics
                    .as_ref()
                    .and_then(|graphics| graphics.pick_sprite_at(point))
            })
        };
        let _ = self.input_sender.send(RuntimeInputEvent::PointerButton {
            pressed,
            inside_stage: stage_pointer.is_some(),
            target_instance: picked.map(|(instance, _)| instance),
            draggable: picked.is_some_and(|(_, draggable)| draggable),
            was_dragged: self.pointer_was_dragged,
            monitor_interaction,
        });
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
                if !self.redraw_pending {
                    self.request_redraw();
                }
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
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    winit::event::MouseScrollDelta::PixelDelta(position) => {
                        self.graphics.as_ref().map_or(0.0, |graphics| {
                            -graphics.window_scroll_pixels_to_list_rows(position.y)
                        })
                    }
                };
                if rows != 0.0
                    && let (Some(pointer), Some(graphics)) =
                        (self.stage_pointer(), self.graphics.as_mut())
                {
                    match graphics.scroll_list_monitor(pointer.0, pointer.1, rows) {
                        Ok(true) => self.request_redraw(),
                        Ok(false) => {}
                        Err(error) => {
                            self.error = Some(format!("failed to scroll list monitor: {error}"));
                        }
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.resize(size);
                }
                self.sync_latest_snapshot();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && self
                        .graphics
                        .as_ref()
                        .is_some_and(GraphicsState::list_item_editor_is_open)
                {
                    let input = match &event.logical_key {
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                            Some(ListEditorInput::Commit)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                            Some(ListEditorInput::Cancel)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                            Some(ListEditorInput::Backspace)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete) => {
                            Some(ListEditorInput::Delete)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowLeft) => {
                            Some(ListEditorInput::Left)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowRight) => {
                            Some(ListEditorInput::Right)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                            Some(ListEditorInput::Home)
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::End) => {
                            Some(ListEditorInput::End)
                        }
                        _ => event.text.as_deref().map(ListEditorInput::Text),
                    };
                    if let (Some(input), Some(graphics)) = (input, self.graphics.as_mut()) {
                        let (changed, events) = graphics.edit_list_item(input);
                        for event in events {
                            let _ = self.input_sender.send(event);
                        }
                        if changed {
                            self.request_redraw();
                        }
                    }
                    return;
                }
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
            WindowEvent::Focused(false) => {
                // avoid leaving a key held after the operating system drops its key-up event.
                let _ = self.input_sender.send(RuntimeInputEvent::ClearKeys);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(position);
                // scratch gui promotes a held pointer to a drag after three css pixels.
                if let Some(start) = self.pointer_down_at
                    && !self.pointer_was_dragged
                {
                    let scale = self
                        .graphics
                        .as_ref()
                        .map_or(1.0, |graphics| graphics.window().scale_factor());
                    let threshold = 3.0 * scale;
                    let dx = position.x - start.x;
                    let dy = position.y - start.y;
                    self.pointer_was_dragged = dx * dx + dy * dy > threshold * threshold;
                }
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
                        let scrollbar_result = self
                            .graphics
                            .as_mut()
                            .map(|graphics| graphics.update_list_scrollbar_hover(Some((x, y))));
                        let slider_result = self
                            .graphics
                            .as_mut()
                            .map(|graphics| graphics.update_variable_slider_hover(Some((x, y))));
                        if let Some(result) = scrollbar_result {
                            match result {
                                Ok(true) => self.request_redraw(),
                                Ok(false) => {}
                                Err(error) => {
                                    self.error = Some(format!(
                                        "failed to update list scrollbar hover: {error}"
                                    ));
                                }
                            }
                        }
                        if let Some(result) = slider_result {
                            match result {
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
                if let Some(graphics) = self.graphics.as_mut() {
                    match graphics.update_list_scrollbar_hover(None) {
                        Ok(true) => self.request_redraw(),
                        Ok(false) => {}
                        Err(error) => {
                            self.error =
                                Some(format!("failed to clear list scrollbar hover: {error}"));
                        }
                    }
                }
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
                            self.pointer_down_at = self.cursor_position;
                            self.pointer_was_dragged = false;
                            self.pointer_suppressed = false;
                            let menu_open = self.dev_mode
                                && self
                                    .graphics
                                    .as_ref()
                                    .is_some_and(GraphicsState::context_menu_is_open);
                            if menu_open {
                                self.pointer_suppressed = true;
                                let mut list_action = None;
                                if let Some(graphics) = self.graphics.as_mut() {
                                    match graphics.activate_variable_context_menu(stage_pointer) {
                                        Ok(action) => {
                                            list_action = action;
                                            self.request_redraw();
                                        }
                                        Err(error) => {
                                            self.error = Some(format!(
                                                "failed to activate variable menu: {error}"
                                            ));
                                        }
                                    }
                                }
                                if let Some(action) = list_action {
                                    if let Err(error) =
                                        handle_list_monitor_file_action(action, &self.input_sender)
                                    {
                                        self.error = Some(error);
                                    }
                                }
                                return;
                            }

                            if let (Some((x, y)), Some(graphics)) =
                                (stage_pointer, self.graphics.as_mut())
                            {
                                match graphics.handle_list_pointer(x, y) {
                                    Ok(response) => {
                                        for event in response.events {
                                            let _ = self.input_sender.send(event);
                                        }
                                        if response.resizing {
                                            self.dragging_variable = true;
                                        }
                                        if response.scrollbar_dragging {
                                            self.dragging_variable = true;
                                        }
                                        if response.consumed {
                                            self.pointer_suppressed = true;
                                            self.request_redraw();
                                            return;
                                        }
                                    }
                                    Err(error) => {
                                        self.error = Some(format!(
                                            "failed to interact with list monitor: {error}"
                                        ));
                                    }
                                }
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
                                self.pointer_suppressed |= hit_monitor;
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
                            if !self.pointer_suppressed {
                                self.send_pointer_button(true, false);
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
                                    if let Err(error) =
                                        graphics.update_list_scrollbar_hover(stage_pointer)
                                    {
                                        self.error = Some(format!(
                                            "failed to update list scrollbar hover: {error}"
                                        ));
                                    }
                                }
                                self.dragging_variable = false;
                                self.dragging_slider = false;
                                self.pointer_suppressed = true;
                                self.request_redraw();
                            }
                            if !self.pointer_suppressed {
                                self.send_pointer_button(false, false);
                            }
                            self.pointer_down_at = None;
                            self.pointer_was_dragged = false;
                            self.pointer_suppressed = false;
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
        let snapshot = self.snapshot();
        let mouse_position = self.cursor_position.and_then(|position| {
            self.graphics
                .as_ref()
                .and_then(|graphics| graphics.window_to_stage_unclamped(position))
                .map(|(x, y)| (x as f64, y as f64))
        });
        let (live_monitor_visible, live_monitors_changed) =
            if let Some(graphics) = self.graphics.as_mut() {
                match graphics.refresh_live_monitors(&snapshot, Instant::now(), mouse_position) {
                    Ok(changed) => (graphics.has_visible_live_monitor(), changed),
                    Err(error) => {
                        self.error = Some(format!("failed to refresh live monitors: {error}"));
                        (false, false)
                    }
                }
            } else {
                (false, false)
            };
        if live_monitors_changed {
            self.request_redraw();
        }
        if live_monitor_visible {
            let vsync_interval = self
                .graphics
                .as_ref()
                .and_then(|graphics| graphics.window().current_monitor())
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .filter(|rate| *rate > 0)
                .map(|rate| Duration::from_secs_f64(1000.0 / rate as f64))
                .unwrap_or(Duration::from_secs_f64(1.0 / 60.0));
            let interval = self.requested_frame_interval.max(vsync_interval);
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + interval));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn handle_list_monitor_file_action(
    action: ListMonitorFileAction,
    input_sender: &mpsc::Sender<RuntimeInputEvent>,
) -> Result<(), String> {
    match action {
        ListMonitorFileAction::Import { key } => {
            let path = rfd::FileDialog::new()
                .add_filter("list files", &["csv", "tsv", "txt"])
                .pick_file();
            let Some(path) = path else {
                return Ok(());
            };
            let contents = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            let items = parse_scratch_list_import(&contents)?;
            let _ = input_sender.send(RuntimeInputEvent::ListReplace { key, items });
        }
        ListMonitorFileAction::Export {
            file_name,
            contents,
        } => {
            let path = rfd::FileDialog::new()
                .add_filter("text file", &["txt"])
                .set_file_name(&file_name)
                .save_file();
            if let Some(path) = path {
                std::fs::write(path, contents).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

fn parse_scratch_list_import(contents: &str) -> Result<Vec<String>, String> {
    let contents = contents.trim_start_matches('\u{feff}');
    let delimiter = detect_list_delimiter(contents);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(contents.as_bytes());
    reader
        .records()
        .map(|record| {
            record
                .map_err(|error| error.to_string())
                .map(|row| row.get(0).map(str::to_owned))
        })
        .filter_map(|record| match record {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn detect_list_delimiter(contents: &str) -> u8 {
    let mut counts = [(b',', 0usize), (b'\t', 0), (b';', 0), (b'|', 0)];
    for line in contents.lines().take(10) {
        let mut quoted = false;
        let mut chars = line.bytes().peekable();
        while let Some(character) = chars.next() {
            if character == b'"' {
                if quoted && chars.peek() == Some(&b'"') {
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            } else if !quoted {
                for (delimiter, count) in &mut counts {
                    if character == *delimiter {
                        *count += 1;
                    }
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map_or(b',', |(delimiter, _)| delimiter)
}

fn scratch_key_name(key: &winit::keyboard::Key) -> Option<String> {
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

fn position_distance(a: PhysicalPosition<f64>, b: PhysicalPosition<f64>) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::{parse_scratch_list_import, scratch_key_name};
    use winit::keyboard::{Key, NamedKey};

    #[test]
    fn translates_winit_keyboard_keys_to_scratch_names() {
        assert_eq!(
            scratch_key_name(&Key::Character("A".into())),
            Some("A".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::Space)),
            Some(" ".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::ArrowLeft)),
            Some("ArrowLeft".into())
        );
        assert_eq!(
            scratch_key_name(&Key::Named(NamedKey::Enter)),
            Some("Enter".into())
        );
    }

    #[test]
    fn list_import_reads_first_csv_or_tsv_column_and_keeps_quoted_values() {
        assert_eq!(
            parse_scratch_list_import("title,value\r\n\"a,b\",2\r\nempty,\r\n").unwrap(),
            vec!["title", "a,b", "empty"]
        );
        assert_eq!(
            parse_scratch_list_import("one\tignored\ntwo\tignored\n").unwrap(),
            vec!["one", "two"]
        );
    }
}
