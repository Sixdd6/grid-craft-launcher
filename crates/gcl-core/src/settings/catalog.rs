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
        /// How the stored number is shown to a user, when the two differ. `None` shows the
        /// stored number itself.
        display: Option<Display>,
    },
    /// A `true`/`false` switch.
    Toggle,
    /// A fixed set of stored tokens, each paired with a display label.
    Choice(&'static [(&'static str, &'static str)]),
    /// Free text, stored and displayed verbatim.
    Text,
}

/// How a slider's stored number is turned into the number a user reads.
///
/// `shown = stored * mul + add`, and `stored = (shown - add) / mul`. Minecraft stores `fov` as
/// a float in `[-1.0, 1.0]`; a player sets degrees, so `fov` carries `mul: 40.0, add: 70.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    /// What the stored value is multiplied by.
    pub mul: f64,
    /// What is added after the multiplication.
    pub add: f64,
    /// How many digits after the decimal point the shown value uses.
    pub decimals: u8,
    /// The unit shown after the number, `""` when it has none.
    pub unit: &'static str,
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
            display: None,
        }
    };
    ($min:expr, $max:expr, $step:expr, $decimals:expr, $display:expr) => {
        Control::Slider {
            min: $min,
            max: $max,
            step: $step,
            decimals: $decimals,
            display: Some($display),
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
        // Stored as a float in [-1.0, 1.0]: `fov:0.0` is 70 degrees and `fov:1.0` is 110.
        // The display scaling turns that back into the degrees a player sets.
        label: "Field of View",
        group: Group::Video,
        control: slider!(
            -1.0,
            1.0,
            0.025,
            3,
            Display {
                mul: 40.0,
                add: 70.0,
                decimals: 0,
                unit: "\u{b0}",
            }
        ),
        default: "0.0",
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
        // A choice, not a slider: 0 means "Auto", not a scale below one. The tokens stay in
        // numeric order, so the picker still reads the way the game's slider does.
        control: Control::Choice(&[
            ("0", "Auto"),
            ("1", "1\u{d7}"),
            ("2", "2\u{d7}"),
            ("3", "3\u{d7}"),
            ("4", "4\u{d7}"),
            ("5", "5\u{d7}"),
            ("6", "6\u{d7}"),
        ]),
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
        // The stored tokens carry their JSON quotes: a real file has `renderClouds:"true"`.
        control: Control::Choice(&[
            ("\"true\"", "On"),
            ("\"fast\"", "Fast"),
            ("\"false\"", "Off"),
        ]),
        default: "\"true\"",
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
    Setting {
        key: "cloudRange",
        label: "Cloud Distance",
        group: Group::Video,
        control: slider!(2.0, 128.0, 1.0, 0),
        default: "128",
    },
    Setting {
        key: "weatherRadius",
        label: "Weather Distance",
        group: Group::Video,
        control: slider!(3.0, 10.0, 1.0, 0),
        default: "10",
    },
    Setting {
        key: "vignette",
        label: "Vignette",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "cutoutLeaves",
        label: "Leaves",
        group: Group::Video,
        control: Control::Toggle,
        default: "true",
    },
    Setting {
        key: "chunkSectionFadeInTime",
        label: "Chunk Fade-In",
        group: Group::Video,
        control: slider!(0.0, 2.0, 0.05, 2),
        default: "0.75",
    },
    Setting {
        key: "glintSpeed",
        label: "Glint Speed",
        group: Group::Video,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.5",
    },
    Setting {
        key: "glintStrength",
        label: "Glint Strength",
        group: Group::Video,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "0.75",
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
    Setting {
        key: "invertXMouse",
        label: "Invert Mouse (Horizontal)",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "toggleAttack",
        label: "Toggle Attack",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "toggleUse",
        label: "Toggle Use",
        group: Group::Controls,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "sprintWindow",
        label: "Sprint Double-Tap Window",
        group: Group::Controls,
        control: slider!(0.0, 10.0, 1.0, 0),
        default: "7",
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
        key: "soundCategory_ui",
        label: "UI Volume",
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
        // Four decimals: a real 26.2 file writes 0.4375, and the stored value must round-trip
        // through `format_value` unchanged.
        control: slider!(0.0, 1.0, 0.0625, 4),
        default: "0.4375",
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
    Setting {
        key: "chatDelay",
        label: "Chat Delay",
        group: Group::Chat,
        control: slider!(0.0, 6.0, 0.1, 1),
        default: "0.0",
    },
    Setting {
        key: "notificationDisplayTime",
        label: "Notification Time",
        group: Group::Chat,
        control: slider!(0.0, 10.0, 0.5, 1),
        default: "1.0",
    },
    Setting {
        key: "highContrast",
        label: "High Contrast",
        group: Group::Chat,
        control: Control::Toggle,
        default: "false",
    },
    Setting {
        key: "darknessEffectScale",
        label: "Darkness Pulsing",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "damageTiltStrength",
        label: "Damage Tilt",
        group: Group::Chat,
        control: slider!(0.0, 1.0, 0.01, 2),
        default: "1.0",
    },
    Setting {
        key: "menuBackgroundBlurriness",
        label: "Menu Background Blur",
        group: Group::Chat,
        control: slider!(0.0, 10.0, 1.0, 0),
        default: "5",
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
        // Quoted in the file, the same as `renderClouds`: `mainHand:"right"`.
        control: Control::Choice(&[("\"right\"", "Right"), ("\"left\"", "Left")]),
        default: "\"right\"",
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
        // Platform-dependent: `true` on Windows, `false` everywhere else. A real Linux 26.2
        // file writes `false`, and any instance's own `options.txt` value wins over this.
        default: "false",
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
        default: "true",
    },
    Setting {
        key: "resourcePacks",
        label: "Resource Packs",
        group: Group::Other,
        control: Control::Text,
        default: "[]",
    },
];

impl Setting {
    /// The number a user reads for a stored slider value.
    ///
    /// A control with no [`Display`] shows the stored number itself, so this is the identity
    /// for every setting but `fov`.
    pub fn to_display(&self, stored: f64) -> f64 {
        match self.control {
            Control::Slider {
                display: Some(display),
                ..
            } => stored * display.mul + display.add,
            _ => stored,
        }
    }

    /// The value to store for a number a user set. The inverse of [`to_display`](Self::to_display).
    pub fn from_display(&self, shown: f64) -> f64 {
        match self.control {
            Control::Slider {
                display: Some(display),
                ..
            } if display.mul != 0.0 => (shown - display.add) / display.mul,
            _ => shown,
        }
    }
}

/// Looks up a setting by its `options.txt` key.
pub fn find(key: &str) -> Option<&'static Setting> {
    CATALOG.iter().find(|s| s.key == key)
}

/// Parses `value` as `setting`'s control expects, validating range or choice membership.
///
/// A `Slider` with `decimals == 0` parses as [`Value::Int`]; any other `Slider` parses as
/// [`Value::Float`]. Both are checked against `[min, max]`, inclusive, and a value that does
/// not parse as a finite number — including `NaN` and the infinities, which `f64::from_str`
/// accepts — is [`Error::BadValue`]. A `Toggle` accepts exactly `true` or `false`.
/// A `Choice` value matches one of the setting's stored tokens, either as stored (`"fast"`) or
/// bare (`fast`), and is returned in the stored form; anything else is [`Error::BadChoice`],
/// whose message lists each bare token with the label it stands for. `Text` accepts anything.
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
                return Err(out_of_range(setting, min, max));
            }
            Ok(Value::Int(parsed))
        }
        Control::Slider { min, max, .. } => {
            let parsed: f64 = value.parse().map_err(|_| Error::BadValue {
                key: setting.key.to_string(),
                value: value.to_string(),
            })?;
            if !parsed.is_finite() {
                return Err(Error::BadValue {
                    key: setting.key.to_string(),
                    value: value.to_string(),
                });
            }
            if parsed < min || parsed > max {
                return Err(out_of_range(setting, min, max));
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
            // A stored token keeps the quotes `options.txt` writes, but a person types the bare
            // word, so both are taken and the stored form is what comes back.
            let found = values
                .iter()
                .find(|(stored, _)| *stored == value || bare(stored) == value);
            match found {
                Some((stored, _)) => Ok(Value::Text((*stored).to_string())),
                None => Err(Error::BadChoice {
                    key: setting.key.to_string(),
                    value: value.to_string(),
                    allowed: allowed_tokens(values),
                }),
            }
        }
        Control::Text => Ok(Value::Text(value.to_string())),
    }
}

/// The token without the quotes `options.txt` stores it with: `"fast"` is typed `fast`.
fn bare(token: &str) -> &str {
    token.trim_matches('"')
}

/// Every token a choice accepts, each with the label it stands for, for an error message.
///
/// The token is bare, the way a person types it, and the label follows in brackets, so
/// `graphicsMode` reads `0 (Fast), 1 (Fancy), 2 (Fabulous)` rather than three bare numbers.
fn allowed_tokens(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(stored, label)| format!("{} ({label})", bare(stored)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The [`Error::OutOfRange`] for a slider, with the bounds named the way a user reads them.
///
/// `fov` is stored as `-1.0..1.0` and shown as `30°..110°`, so its message names both: the
/// degrees a user reads and, in brackets, the numbers `options.txt` holds. A setting with no
/// [`Display`] names its stored bounds once, because the two forms are the same numbers.
fn out_of_range(setting: &Setting, min: f64, max: f64) -> Error {
    let mut low = setting.to_display(min);
    let mut high = setting.to_display(max);
    if low > high {
        std::mem::swap(&mut low, &mut high);
    }
    let scaled = matches!(
        setting.control,
        Control::Slider {
            display: Some(_),
            ..
        }
    );
    Error::OutOfRange {
        key: setting.key.to_string(),
        min,
        max,
        min_shown: shown_bound(setting, low),
        max_shown: shown_bound(setting, high),
        scaled,
    }
}

/// One bound as a user reads it: the shown number at the display's precision, plus its unit.
fn shown_bound(setting: &Setting, shown: f64) -> String {
    match setting.control {
        Control::Slider {
            display: Some(display),
            ..
        } => format!(
            "{:.*}{}",
            usize::from(display.decimals),
            shown,
            display.unit
        ),
        _ => format!("{shown}"),
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
        // The quotes are part of the stored token: a real file holds `mainHand:"left"`.
        let value = parse_value(setting, "\"left\"").expect("parse");
        assert_eq!(value, Value::Text("\"left\"".to_string()));
        assert_eq!(format_value(setting, &value), "\"left\"");
    }

    #[test]
    fn gui_scale_is_a_choice_from_auto_to_six() {
        let setting = find("guiScale").expect("catalog entry");
        let Control::Choice(tokens) = setting.control else {
            panic!("guiScale is a choice, got {:?}", setting.control);
        };
        assert_eq!(
            tokens,
            &[
                ("0", "Auto"),
                ("1", "1\u{d7}"),
                ("2", "2\u{d7}"),
                ("3", "3\u{d7}"),
                ("4", "4\u{d7}"),
                ("5", "5\u{d7}"),
                ("6", "6\u{d7}"),
            ]
        );
        let value = parse_value(setting, "3").expect("parse");
        assert_eq!(value, Value::Text("3".to_string()));
        assert_eq!(format_value(setting, &value), "3");
        let err = parse_value(setting, "7").expect_err("out of the list");
        assert!(matches!(err, Error::BadChoice { .. }), "{err:?}");
    }

    #[test]
    fn an_unquoted_choice_token_is_normalised_to_the_stored_form() {
        for key in ["mainHand", "renderClouds"] {
            let setting = find(key).expect("catalog entry");
            let bare = setting.default.trim_matches('"');
            let value = parse_value(setting, bare).expect("bare token");
            assert_eq!(
                format_value(setting, &value),
                setting.default,
                "{key}: the quotes come back"
            );
        }
        let clouds = find("renderClouds").expect("catalog entry");
        let value = parse_value(clouds, "fast").expect("bare token");
        assert_eq!(value, Value::Text("\"fast\"".to_string()));
    }

    #[test]
    fn a_bad_choice_names_every_token_it_would_take() {
        let setting = find("renderClouds").expect("catalog entry");
        let err = parse_value(setting, "cloudy").expect_err("bad choice");
        let message = err.to_string();
        assert!(
            message.contains("true (On), fast (Fast), false (Off)"),
            "a quoted token is shown bare, with the label it stands for: {message}"
        );
    }

    #[test]
    fn an_out_of_range_slider_names_the_range_a_user_reads_and_the_stored_one() {
        let fov = find("fov").expect("catalog entry");
        let err = parse_value(fov, "90").expect_err("out of range");
        assert_eq!(
            err.to_string(),
            "\"fov\" must be between 30\u{b0} and 110\u{b0} (stored as -1 to 1)"
        );
        let distance = find("renderDistance").expect("catalog entry");
        let err = parse_value(distance, "999").expect_err("out of range");
        assert_eq!(
            err.to_string(),
            "\"renderDistance\" must be between 2 and 32"
        );
    }

    #[test]
    fn fov_is_stored_as_a_fraction_and_shown_as_degrees() {
        let setting = find("fov").expect("catalog entry");
        assert_eq!(setting.default, "0.0");
        assert_eq!(setting.to_display(0.0), 70.0);
        assert_eq!(setting.to_display(1.0), 110.0);
        assert_eq!(setting.to_display(-1.0), 30.0);
        assert_eq!(setting.from_display(70.0), 0.0);
        assert_eq!(setting.from_display(110.0), 1.0);
        assert_eq!(setting.from_display(30.0), -1.0);
        assert_eq!(setting.from_display(setting.to_display(0.25)), 0.25);
    }

    #[test]
    fn a_slider_without_display_scaling_shows_its_stored_value() {
        let setting = find("renderDistance").expect("catalog entry");
        assert_eq!(setting.to_display(16.0), 16.0);
        assert_eq!(setting.from_display(16.0), 16.0);
        let toggle = find("fullscreen").expect("catalog entry");
        assert_eq!(toggle.to_display(1.0), 1.0);
        assert_eq!(toggle.from_display(1.0), 1.0);
    }

    #[test]
    fn only_fov_carries_display_scaling() {
        for setting in CATALOG {
            if let Control::Slider {
                display: Some(_), ..
            } = setting.control
            {
                assert_eq!(setting.key, "fov", "unexpected display scaling");
            }
        }
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
    fn a_bad_choice_message_pairs_each_token_with_its_label() {
        let setting = find("graphicsMode").expect("catalog entry");
        let err = parse_value(setting, "pretty").expect_err("bad choice");
        assert!(
            err.to_string()
                .ends_with("0 (Fast), 1 (Fancy), 2 (Fabulous)"),
            "{err}"
        );
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

    #[test]
    fn parse_value_rejects_a_non_finite_slider_value() {
        let setting = find("gamma").expect("catalog entry");
        for value in ["NaN", "inf", "-inf"] {
            let err = parse_value(setting, value).expect_err("not finite");
            assert!(matches!(err, Error::BadValue { .. }), "{value}: {err:?}");
        }
    }

    #[test]
    fn every_catalog_default_parses_and_round_trips() {
        for setting in CATALOG {
            let value = match parse_value(setting, setting.default) {
                Ok(value) => value,
                Err(err) => panic!("{} default {:?}: {err:?}", setting.key, setting.default),
            };
            assert_eq!(
                format_value(setting, &value),
                setting.default,
                "{} does not round-trip",
                setting.key
            );
            if let Control::Slider { min, max, .. } = setting.control {
                let number = match value {
                    Value::Int(v) => v as f64,
                    Value::Float(v) => v,
                    _ => panic!("{} is a slider but parsed as {value:?}", setting.key),
                };
                assert!(
                    number >= min && number <= max,
                    "{} default {number} is outside [{min}, {max}]",
                    setting.key
                );
            }
        }
    }
}
