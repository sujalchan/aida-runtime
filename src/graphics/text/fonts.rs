use ab_glyph::{Font, FontArc, ScaleFont};
use fontdb::{Database, Family, Query, Weight};

pub(crate) fn load_system_fonts() -> Result<(FontArc, FontArc), String> {
    let mut database = Database::new();
    database.load_system_fonts();
    let regular = load_system_font(&database, false)?;
    let bold = load_system_font(&database, true)?;
    Ok((regular, bold))
}

fn load_system_font(database: &Database, bold: bool) -> Result<FontArc, String> {
    let families = [Family::Name("Arial"), Family::SansSerif];
    let query = Query {
        families: &families,
        weight: if bold {
            Weight::SEMIBOLD
        } else {
            Weight::NORMAL
        },
        ..Default::default()
    };
    let face = database
        .query(&query)
        .or_else(|| database.faces().next().map(|face| face.id))
        .ok_or_else(|| "no system font is available for variable monitors".to_owned())?;
    let bytes = database
        .with_face_data(face, |data, _| data.to_vec())
        .ok_or_else(|| "failed to read the system font for variable monitors".to_owned())?;
    FontArc::try_from_vec(bytes)
        .map_err(|error| format!("failed to parse variable monitor font: {error}"))
}

pub(crate) fn text_width(font: &FontArc, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(size);
    let mut width = 0.0;
    let mut previous = None;
    for character in text.chars() {
        let glyph = scaled.glyph_id(character);
        if let Some(previous) = previous {
            width += scaled.kern(previous, glyph);
        }
        width += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
    width
}
