# options.txt catalog research (2026-09-07)

Source: https://minecraft.wiki/w/Options.txt and https://gist.github.com/moritztim/991ff9aef5af3909f71727579bd98568 (default files).

**Checked against a real file on 2026-09-07**: a fresh Minecraft 26.2 Fabric instance's
`.minecraft/options.txt` (5030 bytes, 26.2, Linux). Every default and token below marked
"real 26.2" came off that file. A 1.20.1 file was captured too but held only three lines, so
1.20.1-only keys still rest on the wiki.

Format: one `key:value` per line, `:` separator, no spaces. Booleans `true`/`false`. Floats
written with a decimal point. `resourcePacks` is a JSON list (`resourcePacks:["vanilla"]`).

**Some string values are JSON-quoted and some are not, and the quotes are part of the stored
value.** Quoted in a real 26.2 file: `renderClouds:"true"`, `mainHand:"right"`,
`preferredGraphicsBackend:"default"`, `graphicsPreset:"custom"`, `inactivityFpsLimit:"afk"`,
`musicToast:"never"`, `sharePresence:"all"`, `soundDevice:""`. Bare: `lang:en_us`, `lastServer:`,
`tutorialStep:movement`. `settings::catalog` therefore stores the quotes in a `Choice` token —
`renderClouds`'s tokens are `"\"true\""`, `"\"fast\""`, `"\"false\""`, not `true`/`fast`/`false`.

Ranges marked VERIFY were not confirmed against a real file; use them as slider bounds and let
the Advanced raw editor override.

Keys the catalog deliberately leaves out: every `key_*` binding and every `modelPart_*` toggle
(a real file holds about sixty of them and each needs its own editor), plus the 26.2 keys that
are pure state rather than settings (`version`, `startedCleanly`, `joinedFirstServer`,
`tutorialStep`, `lastServer`). They still survive a read and write as unknown raw rows.

## Video
| key | type | default | range/values |
|---|---|---|---|
| renderDistance | int | 12 | 2–32 |
| simulationDistance | int | 12 | 5–32 (1.18+) |
| fov | float | 0.0 | -1.0–1.0, step 0.025 (real 26.2) |
| maxFps | int | 120 | 10–260 (260 = unlimited) |
| gamma | float | 0.5 | 0.0–1.0 |
| guiScale | int | 0 | 0 auto, 1–6 VERIFY |
| fullscreen | bool | false | |
| enableVsync | bool | true | |
| graphicsMode | int | 1 | 0 fast, 1 fancy, 2 fabulous |
| particles | int | 0 | 0 all, 1 decreased, 2 minimal |
| renderClouds | quoted string | `"true"` | `"true"`, `"fast"`, `"false"` (real 26.2) |
| entityShadows | bool | true | |
| entityDistanceScaling | float | 1.0 | 0.5–5.0 |
| biomeBlendRadius | int | 2 | 0–7 |
| mipmapLevels | int | 4 | 0–4 |
| bobView | bool | true | |
| screenEffectScale | float | 1.0 | 0.0–1.0 |
| fovEffectScale | float | 1.0 | 0.0–1.0 |
| ao | bool | true | |
| prioritizeChunkUpdates | int | 0 | 0 threaded, 1 semi blocking, 2 fully blocking |
| cloudRange | int | 128 | 2–128 VERIFY (real 26.2 value 128) |
| weatherRadius | int | 10 | 3–10 VERIFY (real 26.2 value 10) |
| vignette | bool | true | new in 26.2 |
| cutoutLeaves | bool | true | new in 26.2 |
| chunkSectionFadeInTime | float | 0.75 | 0.0–2.0 VERIFY (real 26.2 value 0.75) |
| glintSpeed | float | 0.5 | 0.0–1.0 (real 26.2) |
| glintStrength | float | 0.75 | 0.0–1.0 (real 26.2) |

`fov` is the one key whose stored number is not the number a player reads. Minecraft 26.2
stores it as a float in `[-1.0, 1.0]` where `0.0` is 70 degrees and `1.0` is 110, so the catalog
carries a `Display { mul: 40.0, add: 70.0, decimals: 0, unit: "°" }` and the editor shows
`stored * 40 + 70` degrees. `Setting::to_display` and `Setting::from_display` convert both ways;
every other slider has `display: None` and shows its stored number.

`graphicsMode` is gone from 26.2 (replaced by `graphicsPreset:"custom"` and
`preferredGraphicsBackend:"default"`) but is still written by 1.20.1, so the catalog keeps it.

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
| invertXMouse | bool | false | new in 26.2 |
| toggleAttack | bool | false | new in 26.2 |
| toggleUse | bool | false | new in 26.2 |
| sprintWindow | int | 7 | 0–10 VERIFY (real 26.2 value 7) |

## Sound
| key | type | default | range |
|---|---|---|---|
| soundCategory_master, _music, _record, _weather, _block, _hostile, _neutral, _player, _ambient, _voice | float | 1.0 | 0.0–1.0 |
| soundCategory_ui | float | 1.0 | 0.0–1.0 (new in 26.2) |
| showSubtitles | bool | false | |
| directionalAudio | bool | false | |

## Chat and accessibility
| key | type | default | range/values |
|---|---|---|---|
| chatOpacity | float | 1.0 | 0.0–1.0 |
| chatScale | float | 1.0 | 0.0–1.0 |
| chatWidth | float | 1.0 | 0.0–1.0 |
| chatHeightFocused | float | 1.0 | 0.0–1.0 |
| chatHeightUnfocused | float | 0.4375 | 0.0–1.0, step 0.0625 VERIFY (real 26.2 value 0.4375, not the 0.44366196 the wiki lists) |
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
| chatDelay | float | 0.0 | 0.0–6.0 VERIFY (real 26.2 value 0.0) |
| notificationDisplayTime | float | 1.0 | 0.0–10.0 VERIFY (real 26.2 value 1.0) |
| highContrast | bool | false | new in 26.2 |
| darknessEffectScale | float | 1.0 | 0.0–1.0 (real 26.2) |
| damageTiltStrength | float | 1.0 | 0.0–1.0 (real 26.2) |
| menuBackgroundBlurriness | int | 5 | 0–10 VERIFY (real 26.2 value 5) |

## Other
| key | type | default | values |
|---|---|---|---|
| lang | string | en_us | language code |
| mainHand | quoted string | `"right"` | `"right"`, `"left"` (real 26.2) |
| attackIndicator | int | 1 | 0 off, 1 crosshair, 2 hotbar |
| skipMultiplayerWarning | bool | false | |
| telemetryOptInExtra | bool | false | |
| syncChunkWrites | bool | false | `true` on Windows, `false` elsewhere; a real Linux 26.2 file writes `false` |
| useNativeTransport | bool | true | |
| realmsNotifications | bool | true | |
| allowServerListing | bool | true | |
| hideMatchedNames | bool | true | real 26.2 writes `true` |
| resourcePacks | list | [] | JSON array of pack ids |

## VERIFY status and the escape hatch

The ranges above carry a `VERIFY` tag when neither the wiki table nor the captured 26.2 file
pins the bound down: `guiScale` (1–6 beyond `0` for auto), `mouseWheelSensitivity` (0.01–10.0),
`narrator`'s enum order (0 off, 1 all, 2 chat, 3 system), and every 26.2 key whose real value
confirms one point of the range but not its ends — `cloudRange`, `weatherRadius`,
`chunkSectionFadeInTime`, `sprintWindow`, `chatDelay`, `notificationDisplayTime`,
`menuBackgroundBlurriness`, and `chatHeightUnfocused`'s step. `settings::catalog` uses these as slider bounds
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
