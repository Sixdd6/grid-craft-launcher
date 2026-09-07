//! A merged, three-layer view of an instance's (or the launcher's) `options.txt` settings.
//!
//! The settings editor reads a [`Vec<Row>`] built by [`merged`]: one row per known catalog
//! key, in catalog order, then one row per unknown key found in any layer, alphabetically.
//! Each row says which layer supplied its value, so the UI can show an "inherited" marker.

use std::collections::{BTreeMap, BTreeSet};

use super::catalog::{self, Setting};
use super::{Error, OptionsFile};

/// Which layer a [`Row`]'s value came from, in the order a value would win: an override beats
/// a preseed, which beats what is already in the instance's `options.txt`, which beats the
/// catalog's built-in default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// The catalog's built-in default: no layer set this key.
    Default,
    /// `config.toml`'s `game_defaults`: the launcher's preseed for new instances.
    Preseed,
    /// `instance.toml`'s `settings_overrides`: rewritten into `options.txt` on every launch.
    Override,
    /// The instance's `options.txt` on disk, with no preseed or override for this key.
    File,
}

/// One row of the merged settings view.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The `options.txt` key.
    pub key: String,
    /// The value that currently wins for this key, stored exactly as `options.txt` would hold
    /// it.
    pub value: String,
    /// Which layer supplied [`Row::value`].
    pub source: Layer,
    /// The catalog entry for this key, if it is a known one.
    pub setting: Option<&'static Setting>,
}

/// Builds the merged settings view.
///
/// Rows for every [`catalog::CATALOG`] key come first, in catalog order. For each, the value
/// is the first of `overrides` (if given), `preseed`, `current` (if given), or the catalog
/// default that actually holds the key, and [`Row::source`] names which one. Then one row per
/// key that appears in any layer but not in the catalog, sorted alphabetically, with
/// `setting: None` and the source picked by the same precedence (there is no catalog default
/// for an unknown key).
pub fn merged(
    preseed: &BTreeMap<String, String>,
    overrides: Option<&BTreeMap<String, String>>,
    current: Option<&OptionsFile>,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(catalog::CATALOG.len());
    for setting in catalog::CATALOG {
        let (value, source) = resolve(setting.key, preseed, overrides, current)
            .unwrap_or_else(|| (setting.default.to_string(), Layer::Default));
        rows.push(Row {
            key: setting.key.to_string(),
            value,
            source,
            setting: Some(setting),
        });
    }

    let mut unknown_keys: BTreeSet<String> = BTreeSet::new();
    for key in preseed.keys() {
        if catalog::find(key).is_none() {
            unknown_keys.insert(key.clone());
        }
    }
    if let Some(overrides) = overrides {
        for key in overrides.keys() {
            if catalog::find(key).is_none() {
                unknown_keys.insert(key.clone());
            }
        }
    }
    if let Some(current) = current {
        for (key, _) in current.pairs() {
            if catalog::find(key).is_none() {
                unknown_keys.insert(key.to_string());
            }
        }
    }

    for key in unknown_keys {
        // Every key here came from one of the three layers, so `resolve` always finds it.
        if let Some((value, source)) = resolve(&key, preseed, overrides, current) {
            rows.push(Row {
                key,
                value,
                source,
                setting: None,
            });
        }
    }

    rows
}

/// Finds `key`'s value and source among `overrides`, `preseed`, and `current`, in that order
/// of precedence. `None` means no layer holds the key.
fn resolve(
    key: &str,
    preseed: &BTreeMap<String, String>,
    overrides: Option<&BTreeMap<String, String>>,
    current: Option<&OptionsFile>,
) -> Option<(String, Layer)> {
    if let Some(value) = overrides.and_then(|o| o.get(key)) {
        return Some((value.clone(), Layer::Override));
    }
    if let Some(value) = preseed.get(key) {
        return Some((value.clone(), Layer::Preseed));
    }
    if let Some(value) = current.and_then(|f| f.get(key)) {
        return Some((value.to_string(), Layer::File));
    }
    None
}

/// Validates `value` against `setting`'s control: range for a slider, membership for a
/// choice, `true`/`false` for a toggle. Text always passes.
pub fn validate(setting: &Setting, value: &str) -> Result<(), Error> {
    catalog::parse_value(setting, value).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn merged_with_nothing_set_uses_catalog_defaults() {
        let rows = merged(&BTreeMap::new(), None, None);
        assert_eq!(rows.len(), catalog::CATALOG.len());
        let render_distance = rows
            .iter()
            .find(|r| r.key == "renderDistance")
            .expect("row");
        assert_eq!(render_distance.value, "12");
        assert_eq!(render_distance.source, Layer::Default);
    }

    #[test]
    fn merged_layers_override_over_preseed_over_file_over_default() {
        let preseed = map(&[("renderDistance", "16"), ("fov", "90")]);
        let overrides = map(&[("renderDistance", "8")]);
        let current = OptionsFile::parse("fov:100\nmaxFps:30\n");

        let rows = merged(&preseed, Some(&overrides), Some(&current));
        let by_key = |key: &str| rows.iter().find(|r| r.key == key).expect("row").clone();

        // override wins
        let render_distance = by_key("renderDistance");
        assert_eq!(render_distance.value, "8");
        assert_eq!(render_distance.source, Layer::Override);

        // preseed wins over the file
        let fov = by_key("fov");
        assert_eq!(fov.value, "90");
        assert_eq!(fov.source, Layer::Preseed);

        // file wins over the catalog default
        let max_fps = by_key("maxFps");
        assert_eq!(max_fps.value, "30");
        assert_eq!(max_fps.source, Layer::File);

        // nothing set: catalog default
        let gamma = by_key("gamma");
        assert_eq!(gamma.value, "0.5");
        assert_eq!(gamma.source, Layer::Default);
    }

    #[test]
    fn merged_appends_unknown_keys_from_every_layer_alphabetically_after_the_catalog() {
        let preseed = map(&[("zGizmo", "1")]);
        let overrides = map(&[("aWidget", "2")]);
        let current = OptionsFile::parse("mGadget:3\n");

        let rows = merged(&preseed, Some(&overrides), Some(&current));
        let unknown: Vec<&Row> = rows.iter().filter(|r| r.setting.is_none()).collect();
        let keys: Vec<&str> = unknown.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["aWidget", "mGadget", "zGizmo"]);
        assert_eq!(unknown[0].value, "2");
        assert_eq!(unknown[0].source, Layer::Override);
        assert_eq!(unknown[1].value, "3");
        assert_eq!(unknown[1].source, Layer::File);
        assert_eq!(unknown[2].value, "1");
        assert_eq!(unknown[2].source, Layer::Preseed);
        // catalog rows still come first
        assert_eq!(rows.len(), catalog::CATALOG.len() + 3);
        assert!(
            rows[..catalog::CATALOG.len()]
                .iter()
                .all(|r| r.setting.is_some())
        );
    }

    #[test]
    fn merged_dedupes_a_key_present_in_more_than_one_layer_as_unknown() {
        let preseed = map(&[("customKey", "old")]);
        let overrides = map(&[("customKey", "new")]);
        let rows = merged(&preseed, Some(&overrides), None);
        let matches: Vec<&Row> = rows.iter().filter(|r| r.key == "customKey").collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, "new");
        assert_eq!(matches[0].source, Layer::Override);
    }

    #[test]
    fn validate_accepts_an_in_range_value_and_rejects_out_of_range() {
        let setting = catalog::find("renderDistance").expect("catalog entry");
        assert!(validate(setting, "16").is_ok());
        let err = validate(setting, "64").expect_err("out of range");
        assert!(matches!(err, Error::OutOfRange { .. }));
    }

    #[test]
    fn validate_rejects_a_bad_choice() {
        let setting = catalog::find("mainHand").expect("catalog entry");
        let err = validate(setting, "sideways").expect_err("bad choice");
        assert!(matches!(err, Error::BadChoice { .. }));
    }

    #[test]
    fn merged_snapshot_locks_row_order_and_sources() {
        let preseed = map(&[("renderDistance", "16"), ("customPreseed", "1")]);
        let overrides = map(&[("fov", "90"), ("customOverride", "2")]);
        let current = OptionsFile::parse("maxFps:30\ncustomFile:3\n");

        let rows = merged(&preseed, Some(&overrides), Some(&current));
        let summary: Vec<(String, String, String)> = rows
            .iter()
            .map(|r| (r.key.clone(), r.value.clone(), format!("{:?}", r.source)))
            .collect();
        insta::assert_json_snapshot!(summary);
    }
}
