# Browser rows: latest version and install state

Date: 2026-09-08. Status: approved in chat.

## Goal

Each search row shows the newest version for the target instance's Minecraft version and
loader, whether the mod is installed there, and whether the installed copy is older. The row's
button reads Add, Update, or Installed accordingly. Update replaces the installed file.

## Core

- `Launcher::latest_versions(source, hits: &[(project_id, kind)], target: VersionTarget) ->
  Vec<LatestVersion>` where `VersionTarget { minecraft: Option<String>, loader: Loader }` and
  `LatestVersion { project_id, version: Option<Version> }`. Filters as `content::add` does:
  `compatible_loaders` for mods, Minecraft only for other kinds, none when the target has
  neither. Modrinth: one `versions` call per project, at most 4 in flight, results cached in
  memory per (source, project, minecraft, loader) for the launcher's lifetime. CurseForge: the
  search hit's `latestFilesIndexes` (game version + mod loader) answer without a call; when the
  index lacks a match, fall back to the versions call.
- `Launcher::install_state(slug, source, project_id, latest: &Version) -> InstallState`:
  `NotInstalled`, `Installed { version_id, number }`, `Older { installed_number }`. Older when
  the latest version's `published` is after the installed version's `published` (looked up in
  the same versions list), or when the installed id is absent from the list and differs from
  the latest id. Modpack-file entries match by resolved project id or sha1 as `content::add`
  does.
- "Latest" is release-first — the version Add would install — so a newer beta never outranks
  an older release; equal publish times read as `Installed`.
- Update reuses `content::add` with `version: Some(latest.id)` at depth 0.

## UI

- `SearchRow` gains `latest_number`, `latest_id`, `installed_number`, `state` (`"unknown" |
  "none" | "not_installed" | "installed" | "older"`).
- Under the title: "Latest for <mc> <loader>: <number>" ("Latest: <number>" with no target,
  "No version for <mc> <loader>" when none). When installed: "Installed: <number>", with a
  `↑` marker before "Latest" when older.
- Button: `row_install` reads "Add to <instance>" (not installed), "Update" (older; installs
  `latest_id` and refreshes the row), or is replaced by a muted "Installed" label. With no
  target instance the button stays "Add to …" and the state text is omitted. While a row's
  latest is unresolved the line reads "Checking…".
- The latest-version job runs per search page after the results paint, guarded by the search
  generation, and updates rows in place as answers arrive. Changing the target instance re-runs
  the state part only.

## Tests

Core: resolver filters per kind and target, the cache, the CurseForge index path, the newer
rule (dates, absent id). GUI `flow_content`: three hits (installed older, installed current,
not installed) → texts and button states; Update → old file gone, new present, row reads
Installed. Real-input smoke: a search on a target with an installed mod shows a state.

## Out of scope

Bulk update of every row; CurseForge live verification without a key.
