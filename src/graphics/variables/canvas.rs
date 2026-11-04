use std::collections::HashMap;

use ab_glyph::{Font, FontArc, ScaleFont};
use lyon::{
    math::{Point, point},
    path::Path,
    tessellation::{
        BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
    },
};
use tiny_skia;

use super::menu::{
    CONTEXT_MENU_ROW_HEIGHT, CONTEXT_MENU_WIDTH, MonitorContextAction, MonitorContextMenu,
    SLIDER_RANGE_EDITOR_HEIGHT, SLIDER_RANGE_EDITOR_WIDTH,
};
use super::{SliderRangeEditor, Triangle, VariableMonitor};

const SHADOW_COLOR: [u8; 3] = [8, 12, 20];
pub(super) fn tessellate_path(path: &Path, fill_rule: FillRule) -> Result<Vec<Triangle>, String> {
    let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    let mut builder = BuffersBuilder::new(&mut buffers, |vertex: FillVertex<'_>| vertex.position());
    tessellator
        .tessellate_path(
            path,
            &FillOptions::default().with_fill_rule(fill_rule),
            &mut builder,
        )
        .map_err(|error| format!("failed to tessellate variable monitor polygon: {error:?}"))?;
    Ok(buffers
        .indices
        .chunks_exact(3)
        .map(|triangle| {
            [
                buffers.vertices[triangle[0] as usize],
                buffers.vertices[triangle[1] as usize],
                buffers.vertices[triangle[2] as usize],
            ]
        })
        .collect())
}

pub(super) fn draw_context_menu(
    pixmap: &mut tiny_skia::Pixmap,
    menu: &MonitorContextMenu,
    glyph_meshes: &HashMap<(char, bool), Vec<Triangle>>,
    bold_font: &FontArc,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    fill_rounded_rect(
        pixmap,
        menu.left,
        menu.top,
        CONTEXT_MENU_WIDTH,
        CONTEXT_MENU_ROW_HEIGHT * menu.options.len() as f32,
        1.0,
        [67, 67, 67, 255],
        0.0,
        0.0,
        1.0,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;
    fill_rounded_rect(
        pixmap,
        menu.left + 0.25,
        menu.top + 0.25,
        CONTEXT_MENU_WIDTH - 0.5,
        CONTEXT_MENU_ROW_HEIGHT * menu.options.len() as f32 - 0.5,
        0.25,
        [17, 17, 17, 255],
        0.0,
        0.0,
        1.0,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;

    for (index, option) in menu.options.iter().enumerate() {
        let label = match option {
            MonitorContextAction::NormalReadout => "normal readout",
            MonitorContextAction::LargeReadout => "large readout",
            MonitorContextAction::Slider => "slider",
            MonitorContextAction::ChangeSliderRange => "change slider range",
            MonitorContextAction::ImportList => "import",
            MonitorContextAction::ExportList => "export",
            MonitorContextAction::Hide => "hide",
            MonitorContextAction::Dismiss => "",
        };
        let row_top = menu.top + index as f32 * CONTEXT_MENU_ROW_HEIGHT;
        if menu.hovered_item == Some(index) {
            fill_rounded_rect(
                pixmap,
                menu.left + 1.0,
                row_top + 1.0,
                CONTEXT_MENU_WIDTH - 2.0,
                CONTEXT_MENU_ROW_HEIGHT - 2.0,
                0.0,
                [76, 151, 255, 255],
                0.0,
                0.0,
                1.0,
                scale_x,
                scale_y,
                offset_x,
                offset_y,
            )?;
        }
        let path = build_text_path(label, true, glyph_meshes, bold_font);
        draw_text(
            pixmap,
            path.as_ref(),
            0.0,
            0.0,
            [255, 255, 255, 255],
            menu.left + 6.0,
            row_top + 11.0,
            0.46,
            scale_x,
            scale_y,
            offset_x,
            offset_y,
        )?;
    }
    // the only row divider belongs above the final option.
    fill_rounded_rect(
        pixmap,
        menu.left + 1.0,
        menu.top + CONTEXT_MENU_ROW_HEIGHT * (menu.options.len() - 1) as f32,
        CONTEXT_MENU_WIDTH - 2.0,
        0.75,
        0.0,
        [58, 58, 58, 255],
        0.0,
        0.0,
        1.0,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;
    Ok(())
}

pub(super) fn draw_slider_range_editor(
    pixmap: &mut tiny_skia::Pixmap,
    editor: &SliderRangeEditor,
    glyph_meshes: &HashMap<(char, bool), Vec<Triangle>>,
    bold_font: &FontArc,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    fill_rounded_rect(
        pixmap,
        editor.left,
        editor.top,
        SLIDER_RANGE_EDITOR_WIDTH,
        SLIDER_RANGE_EDITOR_HEIGHT,
        4.0,
        [67, 67, 67, 255],
        0.0,
        0.0,
        1.0,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;
    fill_rounded_rect(
        pixmap,
        editor.left + 0.5,
        editor.top + 0.5,
        SLIDER_RANGE_EDITOR_WIDTH - 1.0,
        SLIDER_RANGE_EDITOR_HEIGHT - 1.0,
        3.5,
        [24, 24, 24, 255],
        0.0,
        0.0,
        1.0,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;
    let fields = [
        ("min", editor.min_text.as_str(), editor.editing_min, 22.0),
        ("max", editor.max_text.as_str(), !editor.editing_min, 42.0),
    ];
    for (label, value, selected, y) in fields {
        if selected {
            fill_rounded_rect(
                pixmap,
                editor.left + 5.0,
                editor.top + y - 13.0,
                SLIDER_RANGE_EDITOR_WIDTH - 10.0,
                17.0,
                2.0,
                [55, 103, 177, 255],
                0.0,
                0.0,
                1.0,
                scale_x,
                scale_y,
                offset_x,
                offset_y,
            )?;
        }
        let path = build_text_path(&format!("{label}: {value}"), true, glyph_meshes, bold_font);
        draw_text(
            pixmap,
            path.as_ref(),
            0.0,
            0.0,
            [255, 255, 255, 255],
            editor.left + 10.0,
            editor.top + y,
            0.46,
            scale_x,
            scale_y,
            offset_x,
            offset_y,
        )?;
    }
    let help = build_text_path(
        "tab switch enter apply esc cancel",
        true,
        glyph_meshes,
        bold_font,
    );
    draw_text(
        pixmap,
        help.as_ref(),
        0.0,
        0.0,
        [205, 205, 205, 255],
        editor.left + 7.0,
        editor.top + 63.0,
        0.34,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )?;
    Ok(())
}

pub(super) fn draw_monitor_shadow(
    pixmap: &mut tiny_skia::Pixmap,
    monitor: &VariableMonitor,
    draw_x: f32,
    draw_y: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    fill_rounded_rect(
        pixmap,
        5.0,
        6.0,
        monitor.width,
        monitor.height,
        7.0,
        [SHADOW_COLOR[0], SHADOW_COLOR[1], SHADOW_COLOR[2], 255],
        draw_x,
        draw_y,
        monitor.scale,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )
}

// the shadow shape is cached as a tile. a running-sum box filter softens its
// alpha in linear time, so pointer movement only translates the finished tile.
pub(super) fn blur_shadow_alpha(pixmap: &mut tiny_skia::Pixmap, radius: usize) {
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    if width == 0 || height == 0 || radius == 0 {
        return;
    }

    let source: Vec<u8> = pixmap
        .data()
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .collect();
    let mut horizontal = vec![0_u8; width * height];
    let divisor = (radius * 2 + 1) as u32;

    for y in 0..height {
        let row = y * width;
        let mut sum: u32 = source[row..row + width.min(radius + 1)]
            .iter()
            .map(|alpha| u32::from(*alpha))
            .sum();
        for x in 0..width {
            horizontal[row + x] = ((sum + divisor / 2) / divisor) as u8;
            if x >= radius {
                sum -= u32::from(source[row + x - radius]);
            }
            if x + radius + 1 < width {
                sum += u32::from(source[row + x + radius + 1]);
            }
        }
    }

    let output = pixmap.data_mut();
    for x in 0..width {
        let mut sum: u32 = (0..height.min(radius + 1))
            .map(|y| u32::from(horizontal[y * width + x]))
            .sum();
        for y in 0..height {
            let alpha = ((sum + divisor / 2) / divisor) as u8;
            let pixel = &mut output[(y * width + x) * 4..][..4];
            for channel in 0..3 {
                pixel[channel] =
                    ((u32::from(SHADOW_COLOR[channel]) * u32::from(alpha) + 127) / 255) as u8;
            }
            pixel[3] = alpha;
            if y >= radius {
                sum -= u32::from(horizontal[(y - radius) * width + x]);
            }
            if y + radius + 1 < height {
                sum += u32::from(horizontal[(y + radius + 1) * width + x]);
            }
        }
    }
}

pub(super) fn build_text_path(
    text: &str,
    bold: bool,
    glyph_meshes: &HashMap<(char, bool), Vec<Triangle>>,
    font: &FontArc,
) -> Option<tiny_skia::Path> {
    let font_size = if bold { 18.0 } else { 19.0 };
    let scaled = font.as_scaled(font_size);
    let mut pen_x = 0.0;
    let mut previous = None;
    let mut builder = tiny_skia::PathBuilder::new();
    for character in text.chars() {
        let glyph = scaled.glyph_id(character);
        if let Some(previous) = previous {
            pen_x += scaled.kern(previous, glyph);
        }
        if let Some(mesh) = glyph_meshes.get(&(character, bold)) {
            for triangle in mesh {
                let mut triangle = triangle.map(|vertex| point(pen_x + vertex.x, -vertex.y));
                let area = (triangle[1].x - triangle[0].x) * (triangle[2].y - triangle[0].y)
                    - (triangle[1].y - triangle[0].y) * (triangle[2].x - triangle[0].x);
                if area < 0.0 {
                    triangle.swap(1, 2);
                }
                builder.move_to(triangle[0].x, triangle[0].y);
                builder.line_to(triangle[1].x, triangle[1].y);
                builder.line_to(triangle[2].x, triangle[2].y);
                builder.close();
            }
        }
        pen_x += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
    builder.finish()
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut tiny_skia::Pixmap,
    path: Option<&tiny_skia::Path>,
    start_x: f32,
    baseline_y: f32,
    color: [u8; 4],
    draw_x: f32,
    draw_y: f32,
    monitor_scale: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::from_rgba8(
        color[0], color[1], color[2], color[3],
    ));
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::from_row(
            monitor_scale * scale_x,
            0.0,
            0.0,
            monitor_scale * scale_y,
            (draw_x + start_x * monitor_scale) * scale_x + offset_x,
            (draw_y + baseline_y * monitor_scale) * scale_y + offset_y,
        ),
        None,
    );
    Ok(())
}

pub(super) fn centered_baseline(font: &FontArc, size: f32, center_y: f32) -> f32 {
    let scaled = font.as_scaled(size);
    center_y + (scaled.ascent() + scaled.descent()) * 0.5
}

fn fill_rounded_rect(
    pixmap: &mut tiny_skia::Pixmap,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
    draw_x: f32,
    draw_y: f32,
    monitor_scale: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    if width <= 0.0 || height <= 0.0 {
        return Ok(());
    }
    let radius = radius.min(width * 0.5).min(height * 0.5).max(0.0);
    let k = 0.552_284_8 * radius;
    let mut builder = tiny_skia::PathBuilder::new();
    builder.move_to(x + radius, y);
    builder.line_to(x + width - radius, y);
    builder.cubic_to(
        x + width - radius + k,
        y,
        x + width,
        y + radius - k,
        x + width,
        y + radius,
    );
    builder.line_to(x + width, y + height - radius);
    builder.cubic_to(
        x + width,
        y + height - radius + k,
        x + width - radius + k,
        y + height,
        x + width - radius,
        y + height,
    );
    builder.line_to(x + radius, y + height);
    builder.cubic_to(
        x + radius - k,
        y + height,
        x,
        y + height - radius + k,
        x,
        y + height - radius,
    );
    builder.line_to(x, y + radius);
    builder.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    builder.close();
    let path = builder
        .finish()
        .ok_or_else(|| "failed to build rounded variable monitor path".to_owned())?;
    fill_local_path(
        pixmap,
        &path,
        color,
        draw_x,
        draw_y,
        monitor_scale,
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    )
}

#[allow(clippy::too_many_arguments)]
fn fill_local_path(
    pixmap: &mut tiny_skia::Pixmap,
    path: &tiny_skia::Path,
    color: [u8; 4],
    draw_x: f32,
    draw_y: f32,
    monitor_scale: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<(), String> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::from_rgba8(
        color[0], color[1], color[2], color[3],
    ));
    paint.anti_alias = true;
    pixmap.fill_path(
        path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::from_row(
            monitor_scale * scale_x,
            0.0,
            0.0,
            monitor_scale * scale_y,
            draw_x * scale_x + offset_x,
            draw_y * scale_y + offset_y,
        ),
        None,
    );
    Ok(())
}

pub(super) fn monitor_slider_ratio(monitor: &VariableMonitor) -> f32 {
    monitor
        .value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| {
            ((value - monitor.slider_min) / (monitor.slider_max - monitor.slider_min))
                .clamp(0.0, 1.0) as f32
        })
        .unwrap_or(0.5)
}
