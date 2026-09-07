# options.txt catalog research (2026-09-07)

Source: https://minecraft.wiki/w/Options.txt and https://gist.github.com/moritztim/991ff9aef5af3909f71727579bd98568 (default files).
Format: one `key:value` per line, `:` separator, no spaces. Booleans `true`/`false`. Strings bare (`lang:en_us`). `resourcePacks` is a JSON list (`resourcePacks:["vanilla"]`). Floats written with a decimal point.
Ranges marked VERIFY were not confirmed by the wiki; use them as slider bounds and let the Advanced raw editor override.

## Video
| key | type | default | range/values |
|---|---|---|---|
| renderDistance | int | 12 | 2–32 |
| simulationDistance | int | 12 | 5–32 (1.18+) |
| fov | int | 70 | 30–110 (stored as degrees in 1.20+) |
| maxFps | int | 120 | 10–260 (260 = unlimited) |
| gamma | float | 0.5 | 0.0–1.0 |
| guiScale | int | 0 | 0 auto, 1–6 VERIFY |
| fullscreen | bool | false | |
| enableVsync | bool | true | |
| graphicsMode | int | 1 | 0 fast, 1 fancy, 2 fabulous |
| particles | int | 0 | 0 all, 1 decreased, 2 minimal |
| renderClouds | string | true | true, fast, false |
| entityShadows | bool | true | |
| entityDistanceScaling | float | 1.0 | 0.5–5.0 |
| biomeBlendRadius | int | 2 | 0–7 |
| mipmapLevels | int | 4 | 0–4 |
| bobView | bool | true | |
| screenEffectScale | float | 1.0 | 0.0–1.0 |
| fovEffectScale | float | 1.0 | 0.0–1.0 |
| ao | bool | true | |
| prioritizeChunkUpdates | int | 0 | 0 threaded, 1 semi blocking, 2 fully blocking |

## Controls
| key | type | default | range/values |
|---|---|---|---|
| mouseSensitivity | float | 0.5 | 0.0–1.0 |
| invertYMouse | bool | false | |
| autoJump | bool | false | |
| toggleCrouch | bool | false | |
| toggleSprint | bool | false | |
| rawMouseInput | bool | true | |
| discrete_mouse_scroll | bool | false | |
| mouseWheelSensitivity | float | 1.0 | 0.01–10.0 VERIFY |

## Sound
| key | type | default | range |
|---|---|---|---|
| soundCategory_master, _music, _record, _weather, _block, _hostile, _neutral, _player, _ambient, _voice | float | 1.0 | 0.0–1.0 |
| showSubtitles | bool | false | |
| directionalAudio | bool | false | |

## Chat and accessibility
| key | type | default | range/values |
|---|---|---|---|
| chatOpacity | float | 1.0 | 0.0–1.0 |
| chatScale | float | 1.0 | 0.0–1.0 |
| chatWidth | float | 1.0 | 0.0–1.0 |
| chatHeightFocused | float | 1.0 | 0.0–1.0 |
| chatHeightUnfocused | float | 0.44366196 | 0.0–1.0 |
| chatLineSpacing | float | 0.0 | 0.0–1.0 |
| narrator | int | 0 | 0 off, 1 all, 2 chat, 3 system VERIFY order |
| hideLightningFlashes | bool | false | |
| darkMojangStudiosBackground | bool | false | |
| panoramaScrollSpeed | float | 1.0 | 0.0–1.0 |
| reducedDebugInfo | bool | false | |
| autoSuggestions | bool | true | |
| chatColors | bool | true | |
| chatLinks | bool | true | |
| chatLinksPrompt | bool | true | |
| textBackgroundOpacity | float | 0.5 | 0.0–1.0 |
| backgroundForChatOnly | bool | true | |

## Other
| key | type | default | values |
|---|---|---|---|
| lang | string | en_us | language code |
| mainHand | string | right | right, left |
| attackIndicator | int | 1 | 0 off, 1 crosshair, 2 hotbar |
| skipMultiplayerWarning | bool | false | |
| telemetryOptInExtra | bool | false | |
| syncChunkWrites | bool | platform | |
| useNativeTransport | bool | true | |
| realmsNotifications | bool | true | |
| allowServerListing | bool | true | |
| hideMatchedNames | bool | false | |
| resourcePacks | list | [] | JSON array of pack ids |

## VERIFY status and the escape hatch

Three ranges above carry a `VERIFY` tag because the wiki did not confirm them directly:
`guiScale` (1–6 beyond `0` for auto), `mouseWheelSensitivity` (0.01–10.0), and `narrator`'s
enum order (0 off, 1 all, 2 chat, 3 system). `settings::catalog` uses these as slider bounds
and enum orderings on a best-effort basis; they are not load-bearing for correctness, because
[`crate::settings::doc::validate`] only rejects a value the catalog's `Control` cannot express
(out of range, an unknown enum token). Every other key in this document came directly off the
wiki table and is not VERIFY.

The Advanced raw editor (`Launcher::set_instance_override` / `set_game_default` with a key not
in `CATALOG`, or a value a `Slider`/`Choice` control would reject) is the escape hatch: it
writes any `key:value` pair straight to the override or preseed map with only
[`crate::settings::validate_key`] / [`validate_value`] applied, no catalog bounds check. A
`VERIFY` range being wrong in `catalog.rs` therefore never blocks a user from setting a value
Minecraft actually accepts — it only means the slider control might clamp a wider range than
the game does, until someone confirms the true bound and narrows or widens `catalog.rs`.
