use std::collections::BTreeSet;

use gcl_core::settings::catalog;
use gcl_core::settings::doc::{Layer, Row};
use slint::Model;

use super::*;

/// One merged row for `key`, as the core view would hand it over.
fn row(key: &str, value: &str, source: Layer) -> Row {
    Row {
        key: key.to_string(),
        value: value.to_string(),
        source,
        setting: catalog::find(key),
    }
}

/// The choice labels one row offers, in order.
fn choices(model: &crate::SettingRowModel) -> Vec<String> {
    model
        .choices
        .iter()
        .map(|label| label.to_string())
        .collect()
}

#[test]
fn a_slider_row_carries_its_bounds_step_and_decimals() {
    let model = setting_row(&row("renderDistance", "16", Layer::Preseed), Layer::Preseed);
    assert_eq!(model.control, "slider");
    assert_eq!(model.label, "Render Distance");
    assert_eq!(model.group, "Video");
    assert_eq!(model.minimum, 2.0);
    assert_eq!(model.maximum, 32.0);
    assert_eq!(model.step, 1.0);
    assert_eq!(model.decimals, 0);
    assert_eq!(model.number, 16.0);
    assert_eq!(model.value, "16");
}

#[test]
fn a_float_slider_keeps_its_decimals_and_a_value_outside_the_range_stays_a_text_row() {
    let model = setting_row(&row("gamma", "0.75", Layer::File), Layer::Override);
    assert_eq!(model.control, "slider");
    assert_eq!(model.decimals, 2);
    assert_eq!(model.step, 0.01_f64 as f32);
    assert_eq!(model.number, 0.75);

    // A file can hold anything. A value the slider's range cannot show falls back to the
    // text control, where the user still reads what is stored, rather than being clamped to
    // a bound the file never held.
    let high = setting_row(&row("gamma", "9", Layer::File), Layer::Override);
    assert_eq!(high.control, "text");
    assert_eq!(high.value, "9");

    let low = setting_row(&row("gamma", "-1", Layer::File), Layer::Override);
    assert_eq!(low.control, "text");
    assert_eq!(low.value, "-1");
}

#[test]
fn the_fov_row_is_shown_in_degrees_and_saved_as_the_float_options_txt_holds() {
    // `fov:0.0` is 70 degrees, and the range [-1, 1] is 30 to 110.
    let model = setting_row(&row("fov", "0.0", Layer::Default), Layer::Preseed);
    assert_eq!(model.control, "slider");
    assert_eq!(
        model.number, 70.0,
        "the slider is on the number a player reads"
    );
    assert_eq!(model.minimum, 30.0);
    assert_eq!(model.maximum, 110.0);
    assert_eq!(model.step, 1.0, "0.025 stored is one degree shown");
    assert_eq!(model.display_decimals, 0, "degrees are whole numbers");
    assert_eq!(
        model.decimals, 3,
        "and the stored float keeps its own precision"
    );
    assert_eq!(model.unit, "\u{b0}");
    assert_eq!(
        model.value, "0.0",
        "the row still carries what the file holds"
    );

    // Dragging that slider to 90 degrees saves the float Minecraft reads.
    assert_eq!(slider_stored_value("fov", 90.0), "0.5");
    assert_eq!(slider_stored_value("fov", 70.0), "0.0");
    assert_eq!(slider_stored_value("fov", 110.0), "1.0");
    assert_eq!(slider_stored_value("fov", 30.0), "-1.0");

    // A drag reports where the pointer was let go, not the nearest step. 90.28 degrees is
    // what the label rounds to 90, so 90 degrees is what is saved.
    assert_eq!(slider_stored_value("fov", 90.28), "0.5");
}

#[test]
fn a_slider_with_no_display_scaling_shows_and_saves_the_stored_number() {
    let model = setting_row(&row("renderDistance", "16", Layer::Preseed), Layer::Preseed);
    assert_eq!(model.display_decimals, 0);
    assert_eq!(model.unit, "");
    assert_eq!(model.number, 16.0);

    // An integer slider saves an integer, and a float one keeps its decimals.
    assert_eq!(slider_stored_value("renderDistance", 24.0), "24");
    assert_eq!(slider_stored_value("gamma", 0.75), "0.75");
    // A key the catalog does not know has no scaling to undo.
    assert_eq!(slider_stored_value("madeUpKey", 3.5), "3.5");
}

#[test]
fn a_slider_value_that_does_not_parse_stays_a_text_row() {
    // `bright` is not a number, so no slider position stands for it. The row falls back to
    // the text control, where the value a file really holds is on screen and editable.
    let model = setting_row(&row("gamma", "bright", Layer::File), Layer::Override);
    assert_eq!(model.control, "text");
    assert_eq!(model.value, "bright");
    assert_eq!(
        model.label, "Brightness",
        "it is still the catalog's setting"
    );
    assert_eq!(model.group, "Video");
}

#[test]
fn a_toggle_row_reads_true_and_false() {
    let on = setting_row(&row("fullscreen", "true", Layer::Preseed), Layer::Preseed);
    assert_eq!(on.control, "toggle");
    assert!(on.checked);
    let off = setting_row(&row("fullscreen", "false", Layer::Preseed), Layer::Preseed);
    assert!(!off.checked);
}

#[test]
fn a_choice_row_finds_the_position_of_the_stored_token() {
    let model = setting_row(&row("graphicsMode", "2", Layer::Default), Layer::Preseed);
    assert_eq!(model.control, "choice");
    assert_eq!(choices(&model), vec!["Fast", "Fancy", "Fabulous"]);
    assert_eq!(model.choice_index, 2);
}

#[test]
fn a_choice_token_outside_the_catalog_stays_a_text_row() {
    // A ComboBox on index -1 shows an empty box, and the first arrow key would write over
    // `7` without the user ever seeing it. The text control keeps the token readable.
    let model = setting_row(&row("graphicsMode", "7", Layer::File), Layer::Preseed);
    assert_eq!(model.control, "text");
    assert_eq!(model.value, "7");
    assert_eq!(model.choice_index, -1);
    assert!(choices(&model).is_empty());
}

#[test]
fn an_unknown_key_is_a_text_row_under_advanced() {
    let model = setting_row(&row("customThing", "1", Layer::Preseed), Layer::Preseed);
    assert_eq!(model.control, "text");
    assert_eq!(model.group, ADVANCED);
    assert_eq!(model.label, "customThing");
    assert_eq!(model.value, "1");
}

#[test]
fn only_a_row_from_the_edited_layer_offers_reset() {
    let own = setting_row(&row("fov", "90", Layer::Preseed), Layer::Preseed);
    assert!(own.resettable);
    assert!(!own.inherited);
    assert_eq!(own.source, "preseed");

    let inherited = setting_row(&row("fov", "90", Layer::Preseed), Layer::Override);
    assert!(!inherited.resettable);
    assert!(inherited.inherited);
}

#[test]
fn visible_rows_group_every_row_and_keep_the_group_order() {
    let rows = vec![
        row("renderDistance", "16", Layer::Preseed),
        row("mouseSensitivity", "0.5", Layer::Default),
        row("customThing", "1", Layer::Preseed),
    ];
    let lines = visible_rows(&rows, Layer::Preseed, "", &BTreeSet::new());
    let shape: Vec<(String, String)> = lines
        .iter()
        .map(|line| (line.control.to_string(), line.key.to_string()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("group".to_string(), "Video".to_string()),
            ("slider".to_string(), "renderDistance".to_string()),
            ("group".to_string(), "Controls".to_string()),
            ("slider".to_string(), "mouseSensitivity".to_string()),
            ("group".to_string(), "Advanced".to_string()),
            ("text".to_string(), "customThing".to_string()),
        ]
    );
    assert_eq!(lines[0].choice_index, 1, "the header counts its rows");
    assert!(lines[0].checked, "a group nobody closed is open");
}

#[test]
fn a_closed_group_keeps_its_header_and_drops_its_rows() {
    let rows = vec![row("renderDistance", "16", Layer::Preseed)];
    let collapsed: BTreeSet<String> = ["Video".to_string()].into_iter().collect();
    let lines = visible_rows(&rows, Layer::Preseed, "", &collapsed);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].control, "group");
    assert!(!lines[0].checked);
}

#[test]
fn a_search_matches_key_or_label_and_opens_the_group_it_hits() {
    let rows = vec![
        row("renderDistance", "16", Layer::Preseed),
        row("fov", "90", Layer::Preseed),
    ];
    let collapsed: BTreeSet<String> = ["Video".to_string()].into_iter().collect();
    let lines = visible_rows(&rows, Layer::Preseed, "renderdistance", &collapsed);
    assert_eq!(lines.len(), 2, "the header and the one hit: {lines:?}");
    assert_eq!(lines[1].key, "renderDistance");

    // The label is searched as well: "Field of View" is `fov`.
    let by_label = visible_rows(&rows, Layer::Preseed, "field of", &BTreeSet::new());
    assert_eq!(by_label.len(), 2);
    assert_eq!(by_label[1].key, "fov");

    let nothing = visible_rows(&rows, Layer::Preseed, "nothing here", &BTreeSet::new());
    assert!(nothing.is_empty());
}

#[test]
fn choice_token_maps_a_position_back_to_the_stored_value() {
    assert_eq!(choice_token("graphicsMode", 0), Some("0"));
    assert_eq!(choice_token("graphicsMode", 2), Some("2"));
    // The catalog keeps the JSON quotes a real options.txt writes.
    assert_eq!(choice_token("renderClouds", 1), Some("\"fast\""));
    assert_eq!(choice_token("graphicsMode", 9), None);
    assert_eq!(choice_token("graphicsMode", -1), None);
    assert_eq!(choice_token("renderDistance", 0), None);
    assert_eq!(choice_token("customThing", 0), None);
}

#[test]
fn stored_value_formats_a_known_key_and_passes_anything_else_through() {
    assert_eq!(stored_value("gamma", "0.50"), "0.5");
    assert_eq!(stored_value("renderDistance", "16"), "16");
    // Out of range: the launcher is the one that refuses it, so it is passed through.
    assert_eq!(stored_value("renderDistance", "64"), "64");
    assert_eq!(stored_value("customThing", "whatever"), "whatever");
}

#[test]
fn layer_names_are_the_words_the_source_column_shows() {
    assert_eq!(layer_name(Layer::Default), "default");
    assert_eq!(layer_name(Layer::Preseed), "preseed");
    assert_eq!(layer_name(Layer::File), "file");
    assert_eq!(layer_name(Layer::Override), "override");
}

/// The text the slider's value label shows, as `components/setting-row.slint` builds it:
/// `setting_slider.value.to-fixed(entry.display_decimals) + entry.unit`.
///
/// The binding lives in Slint, so this mirrors it. The rendered label is read back from a
/// real window in `tests/flow_settings.rs`.
fn slider_label(model: &crate::SettingRowModel) -> String {
    format!(
        "{:.*}{}",
        usize::try_from(model.display_decimals).unwrap_or(0),
        model.number,
        model.unit
    )
}

#[test]
fn a_slider_label_reads_in_the_display_unit_and_precision() {
    // A row with a `display`: the label is the degrees, not the stored float, and it is
    // written with the display's own precision rather than the stored `decimals`.
    let fov = setting_row(&row("fov", "0.0", Layer::Default), Layer::Preseed);
    assert_eq!(slider_label(&fov), "70\u{b0}");
    assert_eq!(fov.decimals, 3, "the stored float has three");
    let wide = setting_row(&row("fov", "1.0", Layer::Default), Layer::Preseed);
    assert_eq!(slider_label(&wide), "110\u{b0}");

    // A row with no `display`: the stored number, no unit, at the stored precision.
    let render = setting_row(&row("renderDistance", "16", Layer::Preseed), Layer::Preseed);
    assert_eq!(slider_label(&render), "16");
    let gamma = setting_row(&row("gamma", "0.75", Layer::File), Layer::Override);
    assert_eq!(gamma.display_decimals, gamma.decimals);
    assert_eq!(slider_label(&gamma), "0.75");
}
