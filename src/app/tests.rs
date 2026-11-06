use super::{input::scratch_key_name, list_io::parse_scratch_list_import};
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
