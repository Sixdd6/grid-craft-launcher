//! The known `options.txt` keys: labels, groups, controls, and stored-value parsing.
//!
//! See `docs/research/2026-09-07-options-txt-catalog.md` for the source tables. A key not in
//! [`CATALOG`] is not unknown to Minecraft, only to this catalog: [`crate::settings::doc`]
//! still shows it as a raw row, and the Advanced editor writes it straight through
//! [`crate::settings::validate_key`] / [`crate::settings::validate_value`] with no bounds
//! check.

use super::Error;

/// Where a [`Setting`] appears in the settings editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Render distance, FOV, graphics quality, and other display settings.
    Video,
    /// Mouse and keyboard behavior.
    Controls,
    /// Volume sliders and subtitle/audio toggles.
    Sound,
    /// Chat rendering and accessibility options.
    Chat,
    /// Everything else: language, hand, telemetry, server list.
    Other,
}

/// The widget a setting's stored value is edited with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// A numeric range, stored as an int when `decimals == 0`, else a float.
    Slider {
        /// The lowest value the slider allows, inclusive.
        min: f64,
        /// The highest value the slider allows, inclusive.
        max: f64,
        /// The slider's step size.
        step: f64,
        /// How many digits after the decimal point the stored value uses. `0` means the value
        /// is stored and parsed as an integer.
        decimals: u8,
    },
    /// A `true`/`false` switch.
    Toggle,
    /// A fixed set of stored tokens, each paired with a display label.
    Choice(&'static [(&'static str, &'static str)]),
    /// Free text, stored and displayed verbatim.
    Text,
}

/// One known `options.txt` key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setting {
    /// The `options.txt` key, exactly as Minecraft writes it.
    pub key: &'static str,
    /// A short display label for the settings editor.
    pub label: &'static str,
    /// Which group this setting is shown under.
    pub group: Group,
    /// The control used to edit this setting's value.
    pub control: Control,
    /// The value Minecraft ships with, stored exactly as it would appear in `options.txt`.
    pub default: &'static str,
}

/// A value parsed from a stored `options.txt` string, typed by its setting's [`Control`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A `Slider` value with `decimals == 0`.
    Int(i64),
    /// A `Slider` value with `decimals > 0`.
    Float(f64),
    /// A `Toggle` value.
    Bool(bool),
    /// A `Choice` or `Text` value.
    Text(String),
}

macro_rules! slider {
    ($min:expr, $max:expr, $step:expr, $decimals:expr) => {
        Control::Slider {
            min: $min,
            max: $max,
            step: $step,
            decimals: $decimals,
        }
    };
}

/// Every known `options.txt` key from `docs/research/2026-09-07-options-txt-catalog.md`.
pub const CATALOG: &[Setting] = &[
    // Video
    Setting {
        key: "renderDistance",
        label: "Render Distance",
        group: Group::Video,
        control: slider!(2.0, 32.0, 1.0, 0),
        default: "12",
    },
    Setting {
        key: "simulationDistance",
        label: "Simulation Distance",
        group: Group::Video,
        control: slider!(5.0, 32.0, 1.0, 0),
        default: "12",
    },
    Setting {
        key: "fov",
        label: "Field of View",
        group: Group::Video,
        control: slider!(30.0, 110.0, 1.0, 0),
        default: "70",
    },
    Setting {
        key: "maxFps",
        label: "Max Framerate",
        group: Group::Video,
        control: slider!(10.0, 260.0, 1.0, 0),
        default: "120",
    },
    Setting {
        key: "gamma",
        label: "Brightness",
        group: Group::Video,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.5",
    },
    Setting {
        key: "guiScale",
        label: "GUI Scale",
        group: Group::Video,
        control: slider!(0.0, 6.0, 1.0, 0),
        default: "0",
    },
    Setting {
        key: "fullscreen",
        label: "Fullscreen",
        group: Group::Video,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "enableVsync",
        label: "VSync",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "graphicsMode",
        label: "Graphics",
        group: Group::Video,
        control: Control::Choice(&[("0", "Fast"), ("1", "Fancy"), ("2", "Fabulous")]),
        default: "1",
    },
    Setting {
        key: "particles",
        label: "Particles",
        group: Group::Video,
        control: Control::Choice(&[("0", "All"), ("1", "Decreased"), ("2", "Minimal")]),
        default: "0",
    },
    Setting {
        key: "renderClouds",
        label: "Clouds",
        group: Group::Video,
        control: Control::Choice(&[("true", "On"), ("fast", "Fast"), ("false", "Off")]),
        default: "true",
    },
    Setting {
        key: "entityShadows",
        label: "Entity Shadows",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "entityDistanceScaling",
        label: "Entity Distance",
        group: Group::Video,
        control: slider!(0.5, 5.0, 0.1, 2),
        default: "1.0",
    },
    Setting {
        key: "biomeBlendRadius",
        label: "Biome Blend",
        group: Group::Video,
        control: slider!(0.0, 7.0, 1.0, 0),
        default: "2",
    },
    Setting {
        key: "mipmapLevels",
        label: "Mipmap Levels",
        group: Group::Video,
        control: slider!(0.0, 4.0, 1.0, 0),
        default: "4",
    },
    Setting {
        key: "bobView",
        label: "View Bobbing",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "screenEffectScale",
        label: "Distortion Effects",
        group: Group::Video,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "fovEffectScale",
        label: "FOV Effects",
        group: Group::Video,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "ao",
        label: "Smooth Lighting",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "prioritizeChunkUpdates",
        label: "Chunk Updates",
        group: Group::Video,
        control: Control::Choice(&[
            ("0", "Threaded"),
            ("1", "Semi Blocking"),
            ("2", "Fully Blocking"),
        ]),
        default: "0",
    },
    // Controls
    Setting {
        key: "mouseSensitivity",
        label: "Mouse Sensitivity",
        group: Group::Controls,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.5",
    },
    Setting {
        key: "invertYMouse",
        label: "Invert Mouse",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "autoJump",
        label: "Auto-Jump",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "toggleCrouch",
        label: "Toggle Sneak",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "toggleSprint",
        label: "Toggle Sprint",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "rawMouseInput",
        label: "Raw Mouse Input",
        group: Group::Controls,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "discrete_mouse_scroll",
        label: "Discrete Mouse Wheel",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "mouseWheelSensitivity",
        label: "Mouse Wheel Sensitivity",
        group: Group::Controls,
        control: slider!(0.01, 10.0, 0.01, 2),
        default: "1.0",
    },
    // Sound
    Setting {
        key: "soundCategory_master",
        label: "Master Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_music",
        label: "Music Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_record",
        label: "Jukebox/Note Blocks Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_weather",
        label: "Weather Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_block",
        label: "Blocks Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_hostile",
        label: "Hostile Creatures Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_neutral",
        label: "Friendly Creatures Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_player",
        label: "Players Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_ambient",
        label: "Ambient/Environment Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "soundCategory_voice",
        label: "Voice/Speech Volume",
        group: Group::Sound,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "showSubtitles",
        label: "Show Subtitles",
        group: Group::Sound,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "directionalAudio",
        label: "Directional Audio",
        group: Group::Sound,
        control: Control::Toggle,
        default: "false",
    },
    // Chat and accessibility
    Setting {
        key: "chatOpacity",
        label: "Chat Opacity",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "chatScale",
        label: "Chat Text Size",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "chatWidth",
        label: "Chat Width",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "chatHeightFocused",
        label: "Chat Height (Focused)",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "chatHeightUnfocused",
        label: "Chat Height (Unfocused)",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.44366196",
    },
    Setting {
        key: "chatLineSpacing",
        label: "Chat Line Spacing",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.0",
    },
    Setting {
        key: "narrator",
        label: "Narrator",
        group: Group::Chat,
        control: Control::Choice(&[("0", "Off"), ("1", "All"), ("2", "Chat"), ("3", "System")]),
        default: "0",
    },
    Setting {
        key: "hideLightningFlashes",
        label: "Hide Lightning Flashes",
        group: Group::Chat,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "darkMojangStudiosBackground",
        label: "Dark Mojang Studios Background",
        group: Group::Chat,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "panoramaScrollSpeed",
        label: "Panorama Scroll Speed",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "reducedDebugInfo",
        label: "Reduced Debug Info",
        group: Group::Chat,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "autoSuggestions",
        label: "Command Suggestions",
        group: Group::Chat,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "chatColors",
        label: "Chat Colors",
        group: Group::Chat,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "chatLinks",
        label: "Chat Links",
        group: Group::Chat,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "chatLinksPrompt",
        label: "Prompt on Chat Links",
        group: Group::Chat,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "textBackgroundOpacity",
        label: "Text Background Opacity",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.5",
    },
    Setting {
        key: "backgroundForChatOnly",
        label: "Chat Background",
        group: Group::Chat,
        control: Control::Toggle,
        default: "true",
    },
    // Other
    Setting {
        key: "lang",
        label: "Language",
        group: Group::Other,
        control: Control::Text,
        default: "en_us",
    },
    Setting {
        key: "mainHand",
        label: "Main Hand",
        group: Group::Other,
        control: Control::Choice(&[("right", "Right"), ("left", "Left")]),
        default: "right",
    },
    Setting {
        key: "attackIndicator",
        label: "Attack Indicator",
        group: Group::Other,
        control: Control::Choice(&[("0", "Off"), ("1", "Crosshair"), ("2", "Hotbar")]),
        default: "1",
    },
    Setting {
        key: "skipMultiplayerWarning",
        label: "Skip Multiplayer Warning",
        group: Group::Other,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "telemetryOptInExtra",
        label: "Share Extra Telemetry",
        group: Group::Other,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "syncChunkWrites",
        label: "Sync Chunk Writes",
        group: Group::Other,
        control: Control::Toggle,
        // The wiki lists this default as platform-dependent, not a fixed value; `true` stands
        // in as a starting point, and any instance's actual `options.txt` value wins over it.
        default: "true",
    },
    Setting {
        key: "useNativeTransport",
        label: "Use Native Transport",
        group: Group::Other,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "realmsNotifications",
        label: "Realms Notifications",
        group: Group::Other,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "allowServerListing",
        label: "Allow Server Listing",
        group: Group::Other,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "hideMatchedNames",
        label: "Hide Matched Names",
        group: Group::Other,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "resourcePacks",
        label: "Resource Packs",
        group: Group::Other,
        control: Control::Text,
        default: "[]",
    },
];

/// Looks up a setting by its `options.txt` key.
pub fn find(key: &str) -> Option<&'static Setting> {
    CATALOG.iter().find(|s| s.key == key)
}

/// Parses `value` as `setting`'s control expects, validating range or choice membership.
///
/// A `Slider` with `decimals == 0` parses as [`Value::Int`]; any other `Slider` parses as
/// [`Value::Float`]. Both are checked against `[min, max]`, inclusive, and a value that does
/// not parse as a number is [`Error::BadValue`]. A `Toggle` accepts exactly `true` or `false`.
/// A `Choice` value must match one of the setting's stored tokens exactly, or
/// [`Error::BadChoice`]. `Text` accepts anything.
pub fn parse_value(setting: &Setting, value: &str) -> Result<Value, Error> {
    match setting.control {
        Control::Slider {
            min,
            max,
            decimals: 0,
            ..
        } => {
            let parsed: i64 = value.parse().map_err(|_| Error::BadValue {
                key: setting.key.to_string(),
                value: value.to_string(),
            })?;
            let as_f64 = parsed as f64;
            if as_f64 < min || as_f64 > max {
                return Err(Error::OutOfRange {
                    key: setting.key.to_string(),
                    min,
                    max,
                });
            }
            Ok(Value::Int(parsed))
        }
        Control::Slider { min, max, .. } => {
            let parsed: f64 = value.parse().map_err(|_| Error::BadValue {
                key: setting.key.to_string(),
                value: value.to_string(),
            })?;
            if parsed < min || parsed > max {
                return Err(Error::OutOfRange {
                    key: setting.key.to_string(),
                    min,
                    max,
                });
            }
            Ok(Value::Float(parsed))
        }
        Control::Toggle => match value {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(Error::BadValue {
                key: setting.key.to_string(),
                value: value.to_string(),
            }),
        },
        Control::Choice(values) => {
            if values.iter().any(|(stored, _)| *stored == value) {
                Ok(Value::Text(value.to_string()))
            } else {
                Err(Error::BadChoice {
                    key: setting.key.to_string(),
                    value: value.to_string(),
                })
            }
        }
        Control::Text => Ok(Value::Text(value.to_string())),
    }
}

/// Formats `value` the way Minecraft stores it: ints bare, floats with at least one decimal
/// digit (trailing zeros trimmed past that), booleans as `true`/`false`, and choice or text
/// values verbatim.
pub fn format_value(setting: &Setting, value: &Value) -> String {
    match value {
        Value::Int(v) => v.to_string(),
        Value::Float(v) => {
            let decimals = match setting.control {
                Control::Slider { decimals, .. } => decimals,
                _ => 1,
            };
            format_float(*v, decimals)
        }
        Value::Bool(v) => v.to_string(),
        Value::Text(v) => v.clone(),
    }
}

/// Formats `v` with at least `decimals.max(1)` digits, then trims trailing zeros past the
/// first decimal digit so `1.50` becomes `1.5` but `1.00` stays `1.0`.
fn format_float(v: f64, decimals: u8) -> String {
    let digits = (decimals as usize).max(1);
    let mut s = format!("{v:.digits$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.push('0');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_no_duplicate_keys() {
        let mut keys: Vec<&str> = CATALOG.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        let mut deduped = keys.clone();
        deduped.dedup();
        assert_eq!(keys, deduped, "duplicate key in CATALOG");
    }

    #[test]
    fn every_slider_has_min_less_than_max() {
        for setting in CATALOG {
            if let Control::Slider { min, max, .. } = setting.control {
                assert!(
                    min < max,
                    "{}: min {min} is not less than max {max}",
                    setting.key
                );
            }
        }
    }

    #[test]
    fn find_looks_up_a_known_key_and_misses_an_unknown_one() {
        assert_eq!(
            find("renderDistance").map(|s| s.key),
            Some("renderDistance")
        );
        assert_eq!(find("notAKey"), None);
    }

    #[test]
    fn int_slider_round_trips_format_and_parse() {
        let setting = find("renderDistance").expect("catalog entry");
        let value = parse_value(setting, "16").expect("parse");
        assert_eq!(value, Value::Int(16));
        assert_eq!(format_value(setting, &value), "16");
    }

    #[test]
    fn float_slider_round_trips_format_and_parse() {
        let setting = find("gamma").expect("catalog entry");
        let value = parse_value(setting, "0.75").expect("parse");
        assert_eq!(value, Value::Float(0.75));
        assert_eq!(format_value(setting, &value), "0.75");
    }

    #[test]
    fn float_slider_keeps_one_trailing_zero() {
        let setting = find("gamma").expect("catalog entry");
        let value = parse_value(setting, "1.0").expect("parse");
        assert_eq!(format_value(setting, &value), "1.0");
    }

    #[test]
    fn toggle_round_trips_format_and_parse() {
        let setting = find("fullscreen").expect("catalog entry");
        let value = parse_value(setting, "true").expect("parse");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(format_value(setting, &value), "true");
    }

    #[test]
    fn choice_round_trips_format_and_parse() {
        let setting = find("mainHand").expect("catalog entry");
        let value = parse_value(setting, "left").expect("parse");
        assert_eq!(value, Value::Text("left".to_string()));
        assert_eq!(format_value(setting, &value), "left");
    }

    #[test]
    fn text_round_trips_format_and_parse() {
        let setting = find("lang").expect("catalog entry");
        let value = parse_value(setting, "fr_fr").expect("parse");
        assert_eq!(value, Value::Text("fr_fr".to_string()));
        assert_eq!(format_value(setting, &value), "fr_fr");
    }

    #[test]
    fn parse_value_rejects_an_out_of_range_int_slider() {
        let setting = find("renderDistance").expect("catalog entry");
        let err = parse_value(setting, "64").expect_err("out of range");
        assert!(matches!(err, Error::OutOfRange { min, max, .. } if min == 2.0 && max == 32.0));
    }

    #[test]
    fn parse_value_rejects_an_out_of_range_float_slider() {
        let setting = find("gamma").expect("catalog entry");
        let err = parse_value(setting, "1.5").expect_err("out of range");
        assert!(matches!(err, Error::OutOfRange { .. }));
    }

    #[test]
    fn parse_value_rejects_a_non_numeric_slider_value() {
        let setting = find("renderDistance").expect("catalog entry");
        let err = parse_value(setting, "nope").expect_err("not a number");
        assert!(matches!(err, Error::BadValue { .. }));
    }

    #[test]
    fn parse_value_rejects_a_bad_choice() {
        let setting = find("mainHand").expect("catalog entry");
        let err = parse_value(setting, "sideways").expect_err("bad choice");
        assert!(matches!(err, Error::BadChoice { .. }));
    }

    #[test]
    fn parse_value_rejects_a_bad_toggle() {
        let setting = find("fullscreen").expect("catalog entry");
        let err = parse_value(setting, "yes").expect_err("bad toggle");
        assert!(matches!(err, Error::BadValue { .. }));
    }
}
