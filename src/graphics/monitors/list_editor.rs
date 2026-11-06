use std::sync::Arc;

use crate::graphics::text::text_width;
use crate::runtime::RuntimeInputEvent;
use ab_glyph::{Font, ScaleFont};

use super::{MonitorKind, MonitorVisibility, ScratchValue, VariableLayer, fit_monitor_text};

pub(super) const LIST_ROW_HEIGHT: f32 = 24.0;
pub(super) const LIST_FOOTER_HEIGHT: f32 = 22.0;
pub(super) const LIST_RESIZE_HANDLE_SIZE: f32 = 22.0;
pub(super) const LIST_SCROLLBAR_WIDTH: f32 = 5.0;
pub(super) const LIST_SCROLLBAR_RIGHT_INSET: f32 = 2.0;
pub(super) const LIST_SCROLLBAR_MIN_THUMB_HEIGHT: f32 = 16.0;
pub(super) const LIST_ROW_INDEX_X: f32 = 7.0;
pub(super) const LIST_ROW_VALUE_X: f32 = 20.0;
pub(super) const LIST_ROW_TEXT_INSET: f32 = 6.0;
// keep a small gutter between the index column and the orange value field.
pub(super) const LIST_ROW_INDEX_GAP: f32 = 2.0;
// allow for glyph overhang and floating-point rounding in the index width.
pub(super) const LIST_ROW_INDEX_CLEARANCE: f32 = 1.5;
pub(super) const LIST_CONTENT_TOP: f32 = 27.0;
pub(super) const LIST_EMPTY_TOP_PADDING: f32 = 6.0;
pub(super) const LIST_ROW_FONT_SIZE: f32 = 12.0;
pub(super) const LIST_HEADER_FONT_SIZE: f32 = 14.0;
pub(super) const LIST_LENGTH_FONT_SIZE: f32 = 12.0;
pub(super) const LIST_HEADER_HORIZONTAL_PADDING: f32 = 12.0;
pub(super) const LIST_FOOTER_SIDE_PADDING: f32 = 7.0;

pub(crate) enum ListEditorInput<'a> {
    Text(&'a str),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Commit,
    Cancel,
}

pub(crate) struct ListPointerResponse {
    pub(crate) consumed: bool,
    pub(crate) resizing: bool,
    pub(crate) scrollbar_dragging: bool,
    pub(crate) events: Vec<RuntimeInputEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ListScrollbarGeometry {
    pub(super) track_x: f32,
    pub(super) track_y: f32,
    pub(super) track_width: f32,
    pub(super) track_height: f32,
    pub(super) thumb_y: f32,
    pub(super) thumb_height: f32,
}

pub(crate) enum ListMonitorFileAction {
    Import { key: String },
    Export { file_name: String, contents: String },
}

pub(super) struct ListItemEditor {
    pub(super) monitor_index: usize,
    pub(super) index: usize,
    pub(super) text: String,
    pub(super) caret: usize,
    pub(super) select_all: bool,
    pub(super) appended: bool,
    pub(super) dirty: bool,
}

pub(super) struct ListResize {
    pub(super) monitor_index: usize,
    pub(super) pointer_x: f32,
    pub(super) pointer_y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

#[derive(Clone, Copy)]
pub(super) struct ListScrollbarDrag {
    pub(super) monitor_index: usize,
    pub(super) pointer_offset: f32,
}

impl VariableLayer {
    pub(crate) fn set_monitor_list(
        &mut self,
        key: &str,
        items: Arc<Vec<ScratchValue>>,
    ) -> Option<(usize, bool)> {
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.value_key == key)?;
        if self.monitors[index].kind != MonitorKind::List
            || Arc::ptr_eq(&self.monitors[index].list_items, &items)
        {
            return Some((index, false));
        }
        let old_display_len = self.list_display_len(index);
        let old_visible_count = self.monitors[index].visible_list_row_count(old_display_len);
        let old_visible_start = self.monitors[index]
            .list_scroll_offset
            .min(self.monitors[index].max_list_scroll_offset(old_display_len));
        self.monitors[index].list_items = items;
        let displayed_len = self.list_display_len(index);
        self.monitors[index].list_scroll_offset = self.monitors[index]
            .list_scroll_offset
            .min(self.monitors[index].max_list_scroll_offset(displayed_len));
        self.monitors[index].list_scroll_remainder = 0.0;
        // update row and scrollbar geometry when list size or the visible range changes.
        let shapes_changed = old_display_len != displayed_len
            || old_visible_count != self.monitors[index].visible_list_row_count(displayed_len)
            || old_visible_start
                != self.monitors[index]
                    .list_scroll_offset
                    .min(self.monitors[index].max_list_scroll_offset(displayed_len));
        if self.monitors[index].visibility == MonitorVisibility::Visible {
            if shapes_changed {
                self.mark_gpu_shapes(index);
            }
            self.mark_gpu_text(index);
            Some((index, true))
        } else {
            Some((index, false))
        }
    }

    pub(super) fn list_scrollbar_geometry(&self, index: usize) -> Option<ListScrollbarGeometry> {
        let monitor = self.monitors.get(index)?;
        if monitor.kind != MonitorKind::List {
            return None;
        }
        let list_len = self.list_display_len(index);
        let track_y = LIST_CONTENT_TOP;
        let track_height = monitor.height - LIST_CONTENT_TOP - LIST_FOOTER_HEIGHT;
        let content_height = list_len as f32 * LIST_ROW_HEIGHT;
        let max_offset = monitor.max_list_scroll_offset(list_len);
        if track_height <= 0.0 || content_height <= track_height || max_offset == 0 {
            return None;
        }
        let thumb_height = (track_height * track_height / content_height)
            .max(LIST_SCROLLBAR_MIN_THUMB_HEIGHT)
            .min(track_height);
        let thumb_travel = track_height - thumb_height;
        let scroll_progress = monitor.list_scroll_offset.min(max_offset) as f32 / max_offset as f32;
        Some(ListScrollbarGeometry {
            track_x: monitor.width - LIST_SCROLLBAR_RIGHT_INSET - LIST_SCROLLBAR_WIDTH,
            track_y,
            track_width: LIST_SCROLLBAR_WIDTH,
            track_height,
            thumb_y: track_y + thumb_travel * scroll_progress,
            thumb_height,
        })
    }

    pub(super) fn list_scrollbar_is_visible(&self, index: usize) -> bool {
        self.list_scrollbar_hovered == Some(index)
            || self
                .list_scrollbar_drag
                .as_ref()
                .is_some_and(|drag| drag.monitor_index == index)
    }

    pub(crate) fn update_list_scrollbar_hover(&mut self, pointer: Option<(f32, f32)>) -> bool {
        let next = pointer
            .and_then(|(x, y)| self.hit_test(x, y))
            .filter(|&index| self.monitors[index].kind == MonitorKind::List);
        if next == self.list_scrollbar_hovered {
            return false;
        }
        let previous = self.list_scrollbar_hovered;
        self.list_scrollbar_hovered = next;
        let mut visual_change = false;
        if let Some(previous) = previous
            && self.list_scrollbar_geometry(previous).is_some()
        {
            self.mark_gpu_shapes(previous);
            visual_change = true;
        }
        if let Some(next) = next
            && self.list_scrollbar_geometry(next).is_some()
        {
            self.mark_gpu_shapes(next);
            visual_change = true;
        }
        visual_change
    }

    pub(super) fn set_list_scroll_offset(&mut self, index: usize, offset: usize) -> bool {
        let list_len = self.list_display_len(index);
        let monitor = &mut self.monitors[index];
        let next_offset = offset.min(monitor.max_list_scroll_offset(list_len));
        if next_offset == monitor.list_scroll_offset {
            return false;
        }
        monitor.list_scroll_offset = next_offset;
        monitor.list_scroll_remainder = 0.0;
        // update visible rows and the thumb without touching offscreen list values.
        self.mark_gpu_shapes(index);
        self.mark_gpu_text(index);
        true
    }

    pub(crate) fn scroll_list_at(&mut self, x: f32, y: f32, rows: f32) -> Option<usize> {
        let index = self.hit_test(x, y)?;
        let displayed_len = self.list_display_len(index);
        let monitor = self.monitors.get_mut(index)?;
        if monitor.kind != MonitorKind::List || !rows.is_finite() || rows == 0.0 {
            return None;
        }
        let accumulated = monitor.list_scroll_remainder + rows;
        let whole_rows = accumulated.trunc() as isize;
        monitor.list_scroll_remainder = accumulated - whole_rows as f32;
        if whole_rows == 0 {
            return None;
        }
        let max_offset = monitor.max_list_scroll_offset(displayed_len);
        let offset = (monitor.list_scroll_offset as isize + whole_rows)
            .clamp(0, max_offset as isize) as usize;
        if offset == monitor.list_scroll_offset {
            monitor.list_scroll_remainder = 0.0;
            return None;
        }
        monitor.list_scroll_offset = offset;
        // each visible index can shift its own value field, so refresh both instance sets.
        self.mark_gpu_shapes(index);
        self.mark_gpu_text(index);
        Some(index)
    }

    pub(crate) fn list_editor_is_open(&self) -> bool {
        self.list_editor.is_some()
    }

    pub(crate) fn handle_list_pointer(&mut self, x: f32, y: f32) -> ListPointerResponse {
        let mut events = Vec::new();
        let Some(index) = self.hit_test(x, y) else {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        };
        if self.monitors[index].kind != MonitorKind::List {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        let monitor = &self.monitors[index];
        let local_x = (x - monitor.x) / monitor.scale;
        let local_y = (monitor.y - y) / monitor.scale;
        let width = monitor.width;
        let height = monitor.height;

        if local_y < 24.0 {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_x >= width - LIST_RESIZE_HANDLE_SIZE && local_y >= height - LIST_RESIZE_HANDLE_SIZE
        {
            self.commit_list_editor(&mut events);
            self.list_resize = Some(ListResize {
                monitor_index: index,
                pointer_x: x,
                pointer_y: y,
                width,
                height,
            });
            return ListPointerResponse {
                consumed: true,
                resizing: true,
                scrollbar_dragging: false,
                events,
            };
        }

        if let Some(scrollbar) = self.list_scrollbar_geometry(index)
            && (scrollbar.track_x..=scrollbar.track_x + scrollbar.track_width).contains(&local_x)
            && (scrollbar.track_y..=scrollbar.track_y + scrollbar.track_height).contains(&local_y)
        {
            self.commit_list_editor(&mut events);
            if self.list_scrollbar_hovered != Some(index) {
                if let Some(previous) = self.list_scrollbar_hovered.replace(index) {
                    self.mark_gpu_shapes(previous);
                }
                self.mark_gpu_shapes(index);
            }
            if (scrollbar.thumb_y..=scrollbar.thumb_y + scrollbar.thumb_height).contains(&local_y) {
                self.list_scrollbar_drag = Some(ListScrollbarDrag {
                    monitor_index: index,
                    pointer_offset: local_y - scrollbar.thumb_y,
                });
                self.mark_gpu_shapes(index);
                return ListPointerResponse {
                    consumed: true,
                    resizing: false,
                    scrollbar_dragging: true,
                    events,
                };
            }
            let page = self.monitors[index]
                .list_scroll_row_capacity()
                .saturating_sub(1)
                .max(1);
            let current = self.monitors[index].list_scroll_offset;
            let next = if local_y < scrollbar.thumb_y {
                current.saturating_sub(page)
            } else {
                current.saturating_add(page)
            };
            self.set_list_scroll_offset(index, next);
            return ListPointerResponse {
                consumed: true,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_y >= height - LIST_FOOTER_HEIGHT
            && (LIST_FOOTER_SIDE_PADDING..LIST_FOOTER_SIDE_PADDING + 22.0).contains(&local_x)
        {
            self.commit_list_editor(&mut events);
            let list_len = self.list_display_len(index);
            let key = self.monitors[index].value_key.clone();
            self.list_editor = Some(ListItemEditor {
                monitor_index: index,
                index: list_len + 1,
                text: String::new(),
                caret: 0,
                select_all: true,
                appended: true,
                dirty: false,
            });
            let capacity = self.monitors[index].list_scroll_row_capacity();
            self.monitors[index].list_scroll_offset = (list_len + 1).saturating_sub(capacity);
            self.mark_gpu_shapes(index);
            self.mark_gpu_text(index);
            events.push(RuntimeInputEvent::ListAppend { key });
            return ListPointerResponse {
                consumed: true,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_y >= LIST_CONTENT_TOP && local_y < height - LIST_FOOTER_HEIGHT {
            let monitor = &self.monitors[index];
            let displayed_len = self.list_display_len(index);
            let start = monitor
                .list_scroll_offset
                .min(monitor.max_list_scroll_offset(displayed_len));
            let row = ((local_y - LIST_CONTENT_TOP) / LIST_ROW_HEIGHT).floor() as usize;
            let row_index = start + row + 1;
            // keep pointer hit testing aligned with the dynamically sized value field.
            let value_x = self.list_row_value_x(index, row_index);
            if row_index <= displayed_len && local_x >= value_x && local_x < width - 6.0 {
                if self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index
                        && editor.index == row_index
                        && local_x >= width - 28.0
                }) {
                    self.list_editor = None;
                    let key = self.monitors[index].value_key.clone();
                    events.push(RuntimeInputEvent::ListDeleteItem {
                        key,
                        index: row_index,
                    });
                    self.monitors[index].list_scroll_offset = self.monitors[index]
                        .list_scroll_offset
                        .min(self.monitors[index].max_list_scroll_offset(displayed_len - 1));
                    self.mark_gpu_shapes(index);
                    self.mark_gpu_text(index);
                    return ListPointerResponse {
                        consumed: true,
                        resizing: false,
                        scrollbar_dragging: false,
                        events,
                    };
                }

                if self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index && editor.index == row_index
                }) {
                    let selection_changed = self
                        .list_editor
                        .as_ref()
                        .is_some_and(|editor| editor.select_all);
                    let caret = self.list_caret_from_x(index, local_x, row_index);
                    if let Some(editor) = self.list_editor.as_mut() {
                        editor.caret = caret;
                        editor.select_all = false;
                    }
                    if selection_changed {
                        self.mark_gpu_shapes(index);
                    }
                    self.mark_gpu_text(index);
                    return ListPointerResponse {
                        consumed: true,
                        resizing: false,
                        scrollbar_dragging: false,
                        events,
                    };
                }

                self.commit_list_editor(&mut events);
                let text = self.monitors[index]
                    .list_items
                    .get(row_index - 1)
                    .map(crate::runtime::scratch_string)
                    .unwrap_or_default();
                self.list_editor = Some(ListItemEditor {
                    monitor_index: index,
                    index: row_index,
                    caret: text.len(),
                    text,
                    select_all: true,
                    appended: false,
                    dirty: false,
                });
                self.mark_gpu_text(index);
                return ListPointerResponse {
                    consumed: true,
                    resizing: false,
                    scrollbar_dragging: false,
                    events,
                };
            }
        }

        self.commit_list_editor(&mut events);
        ListPointerResponse {
            consumed: true,
            resizing: false,
            scrollbar_dragging: false,
            events,
        }
    }

    pub(crate) fn edit_list_item(
        &mut self,
        input: ListEditorInput<'_>,
    ) -> (bool, Vec<RuntimeInputEvent>) {
        if matches!(input, ListEditorInput::Commit) {
            let index = self.list_editor.as_ref().map(|editor| editor.monitor_index);
            let mut events = Vec::new();
            self.commit_list_editor(&mut events);
            return (index.is_some(), events);
        }
        if matches!(input, ListEditorInput::Cancel) {
            let editor = self.list_editor.take();
            if let Some(editor) = editor {
                if editor.select_all {
                    self.mark_gpu_shapes(editor.monitor_index);
                }
                let index = editor.monitor_index;
                self.mark_gpu_text(index);
                return (true, Vec::new());
            }
            return (false, Vec::new());
        }
        let Some(editor) = self.list_editor.as_mut() else {
            return (false, Vec::new());
        };
        let index = editor.monitor_index;
        let selection_was_active = editor.select_all;
        let events = Vec::new();
        let changed = match input {
            ListEditorInput::Text(text) if !text.is_empty() => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                }
                editor.text.insert_str(editor.caret, text);
                editor.caret += text.len();
                editor.dirty = true;
                true
            }
            ListEditorInput::Backspace => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                    editor.dirty = true;
                    true
                } else if editor.caret > 0 {
                    let previous = previous_char_boundary(&editor.text, editor.caret);
                    editor.text.replace_range(previous..editor.caret, "");
                    editor.caret = previous;
                    editor.dirty = true;
                    true
                } else {
                    false
                }
            }
            ListEditorInput::Delete => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                    editor.dirty = true;
                    true
                } else if editor.caret < editor.text.len() {
                    let next = next_char_boundary(&editor.text, editor.caret);
                    editor.text.replace_range(editor.caret..next, "");
                    editor.dirty = true;
                    true
                } else {
                    false
                }
            }
            ListEditorInput::Left => {
                editor.caret = if editor.select_all {
                    0
                } else {
                    previous_char_boundary(&editor.text, editor.caret)
                };
                editor.select_all = false;
                true
            }
            ListEditorInput::Right => {
                editor.caret = if editor.select_all {
                    editor.text.len()
                } else {
                    next_char_boundary(&editor.text, editor.caret)
                };
                editor.select_all = false;
                true
            }
            ListEditorInput::Home => {
                editor.caret = 0;
                editor.select_all = false;
                true
            }
            ListEditorInput::End => {
                editor.caret = editor.text.len();
                editor.select_all = false;
                true
            }
            ListEditorInput::Commit | ListEditorInput::Cancel => false,
            ListEditorInput::Text(_) => false,
        };
        if changed {
            if selection_was_active {
                self.mark_gpu_shapes(index);
            }
            self.mark_gpu_text(index);
        }
        (changed, events)
    }

    fn commit_list_editor(&mut self, events: &mut Vec<RuntimeInputEvent>) {
        let Some(editor) = self.list_editor.take() else {
            return;
        };
        if editor.select_all {
            self.mark_gpu_shapes(editor.monitor_index);
        }
        if editor.dirty
            && let Some(monitor) = self.monitors.get(editor.monitor_index)
        {
            events.push(RuntimeInputEvent::ListSetItem {
                key: monitor.value_key.clone(),
                index: editor.index,
                value: editor.text,
            });
        }
        self.mark_gpu_text(editor.monitor_index);
    }

    fn list_caret_from_x(&self, monitor_index: usize, local_x: f32, row_index: usize) -> usize {
        let Some(editor) = self.list_editor.as_ref() else {
            return 0;
        };
        if editor.monitor_index != monitor_index || editor.index != row_index {
            return 0;
        }
        // place the caret from the same value-field edge used by row rendering.
        let start_x = self.list_row_value_x(monitor_index, row_index) + LIST_ROW_TEXT_INSET;
        let target_x = (local_x - start_x).max(0.0);
        let scaled = self.regular_font.as_scaled(LIST_ROW_FONT_SIZE);
        let max_width = self.monitors[monitor_index].width - start_x - 28.0;
        let shown = fit_monitor_text(
            &self.regular_font,
            &editor.text,
            max_width,
            LIST_ROW_FONT_SIZE,
        );
        let mut pen_x = 0.0;
        let mut previous = None;
        for (byte_index, character) in shown.char_indices() {
            let glyph = scaled.glyph_id(character);
            if let Some(prior) = previous {
                pen_x += scaled.kern(prior, glyph);
            }
            if target_x < pen_x + scaled.h_advance(glyph) * 0.5 {
                return byte_index;
            }
            pen_x += scaled.h_advance(glyph);
            previous = Some(glyph);
        }
        shown.len().min(editor.text.len())
    }

    // size each value field from its own row index so shorter indices do not waste space.
    pub(super) fn list_row_value_x(&self, monitor_index: usize, row_index: usize) -> f32 {
        let Some(monitor) = self.monitors.get(monitor_index) else {
            return LIST_ROW_VALUE_X;
        };
        let index_text = row_index.to_string();
        let index_width =
            text_width(&self.bold_font, &index_text, LIST_ROW_FONT_SIZE) + LIST_ROW_INDEX_CLEARANCE;
        let available_index_width =
            (monitor.width - LIST_ROW_INDEX_X - 6.0 - LIST_ROW_INDEX_GAP).max(0.0);
        let fitted_index_width = index_width.min(available_index_width);
        (LIST_ROW_INDEX_X + fitted_index_width + LIST_ROW_INDEX_GAP)
            .max(LIST_ROW_VALUE_X)
            .min((monitor.width - 6.0).max(0.0))
    }
}

fn previous_char_boundary(text: &str, caret: usize) -> usize {
    text[..caret.min(text.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_char_boundary(text: &str, caret: usize) -> usize {
    let caret = caret.min(text.len());
    text[caret..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| caret + offset)
}
