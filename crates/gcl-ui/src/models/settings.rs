//! Converters from the core settings view ([`gcl_core::settings::doc::Row`]) to the rows the
//! typed settings editor shows.
//!
//! Pure value conversion plus the two rules the editor needs on top of the core view: which
//! group a row belongs to, and which rows are visible for a given search and set of closed
//! groups. Nothing here calls the launcher.

use std::collections::BTreeSet;

use gcl_core::settings::catalog::{self, Control, Group};
use gcl_core::settings::doc::{Layer, Row};
use slint::{ModelRc, SharedString, VecModel};

use crate::SettingRowModel;

/// The group an unknown key is shown under.
pub const ADVANCED: &str = "Advanced";

/// Every group, in the order the editor shows them.
pub const GROUPS: [&str; 6] = ["Video", "Controls", "Sound", "Chat", "Other", ADVANCED];

/// The `control` value that marks a group header rather than a setting.
pub const HEADER: &str = "group";

/// The group name for one merged row: its catalog group, or [`ADVANCED`] when the key is not
/// in the catalog.
pub fn group_of(row: &Row) -> &'static str {
    match row.setting.map(|setting| setting.group) {
        Some(Group::Video) => "Video",
        Some(Group::Controls) => "Controls",
        Some(Group::Sound) => "Sound",
        Some(Group::Chat) => "Chat",
        Some(Group::Other) => "Other",
        None => ADVANCED,
    }
}

/// What the editor calls one layer: the word shown in a row's source column.
pub fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Default => "default",
        Layer::Preseed => "preseed",
        Layer::File => "file",
        Layer::Override => "override",
    }
}

/// Builds the editor row for one merged row.
///
/// `edited` is the layer the open screen writes: the preseed on the settings screen, the
/// override on an instance. A row whose value comes from that layer is the only kind that
/// offers Reset, and every other row is marked inherited.
pub fn setting_row(row: &Row, edited: Layer) -> SettingRowModel {
    let resettable = row.source == edited;
    let mut model = SettingRowModel {
        key: row.key.as_str().into(),
        label: row.key.as_str().into(),
        group: group_of(row).into(),
        control: "text".into(),
        value: row.value.as_str().into(),
        number: 0.0,
        minimum: 0.0,
        maximum: 0.0,
        step: 1.0,
        decimals: 0,
        checked: false,
        choices: ModelRc::new(VecModel::from(Vec::<SharedString>::new())),
        choice_index: -1,
        source: layer_name(row.source).into(),
        inherited: !resettable,
        resettable,
    };
    let Some(setting) = row.setting else {
        return model;
    };
    model.label = setting.label.into();
    match setting.control {
        // A stored value the catalog's own range cannot hold is left on the text control:
        // a slider has nowhere to put it, and silently snapping it to `min` would hide a
        // value the file really holds.
        Control::Slider {
            min,
            max,
            step,
            decimals,
        } => {
            let Ok(parsed) = row.value.parse::<f64>() else {
                return model;
            };
            model.control = "slider".into();
            model.minimum = min as f32;
            model.maximum = max as f32;
            model.step = step as f32;
            model.decimals = i32::from(decimals);
            model.number = parsed.clamp(min, max) as f32;
        }
        Control::Toggle => {
            model.control = "toggle".into();
            model.checked = row.value == "true";
        }
        // The same rule for a token no option in the catalog holds: a ComboBox on `-1`
        // shows an empty box, and the first arrow key would overwrite the real token, so
        // the row keeps the text control and the token stays readable.
        Control::Choice(values) => {
            let Some(index) = values.iter().position(|(stored, _)| *stored == row.value) else {
                return model;
            };
            model.control = "choice".into();
            let labels: Vec<SharedString> =
                values.iter().map(|(_, label)| (*label).into()).collect();
            model.choices = ModelRc::new(VecModel::from(labels));
            model.choice_index = index as i32;
        }
        Control::Text => model.control = "text".into(),
    }
    model
}

/// Builds the header line for one group. `count` is how many settings it holds right now.
pub fn group_header(name: &str, expanded: bool, count: usize) -> SettingRowModel {
    SettingRowModel {
        key: name.into(),
        label: name.into(),
        group: name.into(),
        control: HEADER.into(),
        value: SharedString::new(),
        number: 0.0,
        minimum: 0.0,
        maximum: 0.0,
        step: 1.0,
        decimals: 0,
        checked: expanded,
        choices: ModelRc::new(VecModel::from(Vec::<SharedString>::new())),
        choice_index: i32::try_from(count).unwrap_or(i32::MAX),
        source: SharedString::new(),
        inherited: false,
        resettable: false,
    }
}

/// Whether one merged row matches a search: its key or its label holds `query`, ignoring
/// case. An empty query matches everything.
pub fn matches(row: &Row, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    let label = row.setting.map(|setting| setting.label).unwrap_or_default();
    row.key.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
}

/// Builds the visible lines: a header for every group that has a matching row, then that
/// group's rows while the group is open.
///
/// A search opens every group it matches in, so a hit is never hidden behind a closed one.
pub fn visible_rows(
    rows: &[Row],
    edited: Layer,
    query: &str,
    collapsed: &BTreeSet<String>,
) -> Vec<SettingRowModel> {
    let query = query.trim();
    let searching = !query.is_empty();
    let mut lines = Vec::new();
    for group in GROUPS {
        let matched: Vec<&Row> = rows
            .iter()
            .filter(|row| group_of(row) == group && matches(row, query))
            .collect();
        if matched.is_empty() {
            continue;
        }
        let expanded = searching || !collapsed.contains(group);
        lines.push(group_header(group, expanded, matched.len()));
        if expanded {
            lines.extend(matched.into_iter().map(|row| setting_row(row, edited)));
        }
    }
    lines
}

/// The stored token one choice position holds, for a key the catalog knows.
///
/// `None` when the key is unknown, its control is not a choice, or the position is outside
/// the list, so a stray index can never be written as a value.
pub fn choice_token(key: &str, index: i32) -> Option<&'static str> {
    let setting = catalog::find(key)?;
    let Control::Choice(values) = setting.control else {
        return None;
    };
    let index = usize::try_from(index).ok()?;
    values.get(index).map(|(stored, _)| *stored)
}

/// The text a control's value is stored as, formatted the way `options.txt` holds it.
///
/// A known key is parsed and formatted back through the catalog, so a slider that sends
/// `1.20` saves as `1.2`. Anything the catalog rejects, and every unknown key, is passed
/// through as typed: the launcher validates it again and reports what is wrong.
pub fn stored_value(key: &str, text: &str) -> String {
    let Some(setting) = catalog::find(key) else {
        return text.to_string();
    };
    match catalog::parse_value(setting, text) {
        Ok(value) => catalog::format_value(setting, &value),
        Err(_) => text.to_string(),
    }
}

#[cfg(test)]
mod tests;
