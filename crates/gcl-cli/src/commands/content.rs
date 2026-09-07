//! `gcl content`: search a source, install into an instance, and manage what is installed.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::content::{AddOutcome, AddRequest, ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, ContentKind};
use gcl_core::sources::{SearchQuery, SourceId};
use serde::Serialize;

use crate::commands::instance::LoaderArg;
use crate::output::{Format, print_json, print_table};

/// Exit code of a command that left downloads for the user to fetch by hand.
pub const EXIT_MANUAL_PENDING: u8 = 3;

/// Subcommands under `gcl content`.
#[derive(Subcommand)]
pub enum ContentCommand {
    /// Search a source for projects.
    Search {
        /// Free-text search string.
        text: String,
        /// Source to search: modrinth or curseforge.
        #[arg(long, default_value = "modrinth")]
        source: String,
        /// Restrict to one kind: mod, resourcepack, shader, datapack, world.
        #[arg(long = "type", value_name = "KIND")]
        kind: Option<String>,
        /// Restrict to one Minecraft version.
        #[arg(long, value_name = "VERSION")]
        mc: Option<String>,
        /// Restrict to one loader.
        #[arg(long, value_enum)]
        loader: Option<LoaderArg>,
        /// Maximum number of hits.
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Offset into the result set.
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Install a project, and its required dependencies, into an instance.
    Add {
        /// Slug of the instance to install into.
        slug: String,
        /// Source to resolve the project at.
        #[arg(long)]
        source: String,
        /// Project id or slug at the source.
        #[arg(long, value_name = "ID|SLUG")]
        project: String,
        /// Version id or number to pin. The newest compatible version wins without one.
        #[arg(long, value_name = "ID|NUMBER")]
        version: Option<String>,
        /// World a data pack installs into.
        #[arg(long, value_name = "NAME")]
        world: Option<String>,
    },
    /// List what an instance has installed.
    List {
        /// Slug of the instance.
        slug: String,
    },
    /// Delete one installed project from disk and from `instance.toml`.
    Remove {
        /// Slug of the instance.
        slug: String,
        /// Project id of the entry to delete.
        project_id: String,
    },
    /// Enable one installed project.
    Enable {
        /// Slug of the instance.
        slug: String,
        /// Project id of the entry to enable.
        project_id: String,
    },
    /// Disable one installed project, which renames its file.
    Disable {
        /// Slug of the instance.
        slug: String,
        /// Project id of the entry to disable.
        project_id: String,
    },
    /// List the installed content that has a newer compatible version.
    Update {
        /// Slug of the instance.
        slug: String,
        /// Install every update instead of only listing them.
        #[arg(long)]
        apply: bool,
    },
    /// List the downloads the user must still fetch by hand.
    Pending {
        /// Slug of the instance.
        slug: String,
    },
    /// Install a hand-downloaded file, satisfying one pending download.
    ImportFile {
        /// Slug of the instance.
        slug: String,
        /// Path of the file that was downloaded by hand.
        path: PathBuf,
        /// Kind of content the file is: mod, resourcepack, shader, datapack, world.
        /// Defaults to the kind the pending download recorded.
        #[arg(long)]
        kind: Option<String>,
        /// Project id of the pending download, when the file name does not match.
        #[arg(long, value_name = "ID")]
        project: Option<String>,
        /// World a data pack installs into, overriding the one the pending download carries.
        #[arg(long, value_name = "NAME")]
        world: Option<String>,
    },
}

/// One installed entry as the CLI reports it.
#[derive(Serialize)]
struct ContentRow {
    project_id: String,
    version_id: String,
    file_name: String,
    kind: String,
    source: String,
    enabled: bool,
}

impl From<&ContentEntry> for ContentRow {
    fn from(entry: &ContentEntry) -> ContentRow {
        ContentRow {
            project_id: entry.project_id.clone(),
            version_id: entry.version_id.clone(),
            file_name: entry.file_name.clone(),
            kind: entry.kind.to_string(),
            source: entry.source.clone(),
            enabled: entry.enabled,
        }
    }
}

/// One pending hand download as the CLI reports it.
#[derive(Serialize)]
struct ManualRow {
    source: String,
    kind: String,
    project_id: String,
    version_id: String,
    file_name: String,
    page_url: String,
    sha1: Option<String>,
    fingerprint: Option<u32>,
}

impl From<&ManualDownload> for ManualRow {
    fn from(manual: &ManualDownload) -> ManualRow {
        ManualRow {
            source: manual.source.to_string(),
            kind: manual.kind.to_string(),
            project_id: manual.project_id.clone(),
            version_id: manual.version_id.clone(),
            file_name: manual.file_name.clone(),
            page_url: manual.page_url.clone(),
            sha1: manual.sha1.clone(),
            fingerprint: manual.fingerprint,
        }
    }
}

/// An [`AddOutcome`] as the CLI reports it in JSON.
#[derive(Serialize)]
struct AddRow {
    installed: Vec<ContentRow>,
    skipped: Vec<String>,
    manual: Vec<ManualRow>,
}

impl From<&AddOutcome> for AddRow {
    fn from(outcome: &AddOutcome) -> AddRow {
        AddRow {
            installed: outcome.installed.iter().map(ContentRow::from).collect(),
            skipped: outcome.skipped.clone(),
            manual: outcome.manual.iter().map(ManualRow::from).collect(),
        }
    }
}

/// One available update as the CLI reports it.
#[derive(Serialize)]
struct UpdateRow {
    project_id: String,
    from_version: String,
    to_version: String,
    file_name: String,
}

impl From<&UpdateCandidate> for UpdateRow {
    fn from(candidate: &UpdateCandidate) -> UpdateRow {
        UpdateRow {
            project_id: candidate.entry.project_id.clone(),
            from_version: candidate.entry.version_id.clone(),
            to_version: candidate.new.id.clone(),
            file_name: candidate.entry.file_name.clone(),
        }
    }
}

/// Parses a source name and checks the launcher has that source configured.
///
/// A source the launcher turned off, which today is CurseForge without an API key, is an
/// error rather than an empty result: the user asked for it by name.
pub fn resolve_source(launcher: &Launcher, name: &str) -> Result<SourceId> {
    let Some(id) = SourceId::parse(name) else {
        bail!("unknown source {name} (use modrinth or curseforge)");
    };
    match launcher.source(id) {
        Ok(_) => Ok(id),
        Err(gcl_core::Error::Sources(gcl_core::sources::Error::Disabled { reason, .. })) => {
            bail!("{id}: disabled ({reason})")
        }
        Err(err) => Err(err.into()),
    }
}

/// Parses a content kind as spelled on the command line.
fn parse_kind(text: &str) -> Result<ContentKind> {
    match ContentKind::parse(text) {
        Some(kind) => Ok(kind),
        None => bail!("unknown content type {text} (mod, resourcepack, shader, datapack, world)"),
    }
}

/// Runs one `gcl content` subcommand. Returns the exit code the CLI should end with.
pub fn run(launcher: &Launcher, format: Format, command: ContentCommand) -> Result<ExitCode> {
    match command {
        ContentCommand::Search {
            text,
            source,
            kind,
            mc,
            loader,
            limit,
            offset,
        } => {
            let id = resolve_source(launcher, &source)?;
            let query = SearchQuery {
                text,
                kind: kind.as_deref().map(parse_kind).transpose()?,
                minecraft: mc,
                loader: loader.map(Into::into),
                offset,
                limit,
            };
            let page = launcher.search(id, &query)?;
            match format {
                Format::Json => print_json(&page)?,
                Format::Text => {
                    let rows: Vec<Vec<String>> = page
                        .hits
                        .iter()
                        .map(|hit| {
                            vec![
                                hit.source.to_string(),
                                hit.project_id.clone(),
                                hit.slug.clone(),
                                hit.title.clone(),
                                hit.downloads.to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["SOURCE", "ID", "SLUG", "TITLE", "DOWNLOADS"], &rows);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ContentCommand::Add {
            slug,
            source,
            project,
            version,
            world,
        } => {
            let id = resolve_source(launcher, &source)?;
            let outcome = launcher.add_content(
                &slug,
                AddRequest {
                    source: id,
                    project,
                    version,
                    kind: None,
                    world,
                },
            )?;
            report_add(format, &outcome)
        }
        ContentCommand::List { slug } => {
            let entries = launcher.list_content(&slug)?;
            let rows: Vec<ContentRow> = entries.iter().map(ContentRow::from).collect();
            match format {
                Format::Json => print_json(&rows)?,
                Format::Text => {
                    let cells: Vec<Vec<String>> = rows
                        .iter()
                        .map(|row| {
                            vec![
                                row.project_id.clone(),
                                row.version_id.clone(),
                                row.file_name.clone(),
                                row.kind.clone(),
                                row.enabled.to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["PROJECT", "VERSION", "FILE", "KIND", "ENABLED"], &cells);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ContentCommand::Remove { slug, project_id } => {
            launcher.remove_content(&slug, &project_id)?;
            match format {
                Format::Json => print_json(&serde_json::json!({ "removed": project_id }))?,
                Format::Text => println!("removed {project_id}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        ContentCommand::Enable { slug, project_id } => {
            set_enabled(launcher, format, &slug, &project_id, true)
        }
        ContentCommand::Disable { slug, project_id } => {
            set_enabled(launcher, format, &slug, &project_id, false)
        }
        ContentCommand::Update { slug, apply } => {
            let candidates = launcher.check_updates(&slug)?;
            if !apply {
                let rows: Vec<UpdateRow> = candidates.iter().map(UpdateRow::from).collect();
                match format {
                    Format::Json => print_json(&rows)?,
                    Format::Text => {
                        let cells: Vec<Vec<String>> = rows
                            .iter()
                            .map(|row| {
                                vec![
                                    row.project_id.clone(),
                                    row.from_version.clone(),
                                    row.to_version.clone(),
                                    row.file_name.clone(),
                                ]
                            })
                            .collect();
                        print_table(&["PROJECT", "FROM", "TO", "FILE"], &cells);
                    }
                }
                return Ok(ExitCode::SUCCESS);
            }
            let outcomes = launcher.apply_updates(&slug, &candidates)?;
            let merged = merge(&outcomes);
            report_add(format, &merged)
        }
        ContentCommand::Pending { slug } => {
            let pending = launcher.pending_manual(&slug)?;
            let rows: Vec<ManualRow> = pending.iter().map(ManualRow::from).collect();
            match format {
                Format::Json => print_json(&rows)?,
                Format::Text => {
                    for row in &rows {
                        println!("{} ({}) -> {}", row.file_name, row.kind, row.page_url);
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ContentCommand::ImportFile {
            slug,
            path,
            kind,
            project,
            world,
        } => {
            let pending = launcher.pending_manual(&slug)?;
            let Some(matched) = pick_pending(&pending, &path, project.as_deref()) else {
                bail!("no pending download matches");
            };
            // Without `--kind` the pending entry's own kind is used: it is the kind the add
            // that produced the entry resolved, so it is right unless the user says otherwise.
            let kind = match kind {
                Some(text) => parse_kind(&text)?,
                None => matched.kind,
            };
            // `--world` wins over the world the pending entry recorded, so a data pack
            // can be dropped into a world the original request did not name.
            let mut matched = matched.clone();
            if world.is_some() {
                matched.world = world;
            }
            let entry = launcher.import_manual_file(&slug, &matched, &path, kind)?;
            match format {
                Format::Json => print_json(&ContentRow::from(&entry))?,
                Format::Text => println!("imported {} ({})", entry.file_name, entry.project_id),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Enables or disables one entry and prints the path its file now has.
fn set_enabled(
    launcher: &Launcher,
    format: Format,
    slug: &str,
    project_id: &str,
    enabled: bool,
) -> Result<ExitCode> {
    let path = launcher.set_content_enabled(slug, project_id, enabled)?;
    let verb = if enabled { "enabled" } else { "disabled" };
    match format {
        Format::Json => print_json(&serde_json::json!({
            "project_id": project_id,
            "enabled": enabled,
            "path": path.display().to_string(),
        }))?,
        Format::Text => println!("{verb} {project_id} ({})", path.display()),
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints an [`AddOutcome`] and returns [`EXIT_MANUAL_PENDING`] when a hand download remains.
fn report_add(format: Format, outcome: &AddOutcome) -> Result<ExitCode> {
    match format {
        Format::Json => print_json(&AddRow::from(outcome))?,
        Format::Text => {
            for entry in &outcome.installed {
                println!("installed {} ({})", entry.file_name, entry.project_id);
            }
            for project in &outcome.skipped {
                println!("already installed {project}");
            }
            for manual in &outcome.manual {
                println!(
                    "manual download {} -> {}",
                    manual.file_name, manual.page_url
                );
            }
        }
    }
    if outcome.manual.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MANUAL_PENDING))
    }
}

/// Folds several [`AddOutcome`]s into one, so `content update --apply` reports once.
fn merge(outcomes: &[AddOutcome]) -> AddOutcome {
    let mut merged = AddOutcome::default();
    for outcome in outcomes {
        merged.installed.extend(outcome.installed.iter().cloned());
        merged.skipped.extend(outcome.skipped.iter().cloned());
        merged.manual.extend(outcome.manual.iter().cloned());
    }
    merged
}

/// Picks the pending download a hand-imported file satisfies.
///
/// `--project` wins when it is given. Without one the file's own name has to equal the
/// pending entry's `file_name`, so a file dropped in under another name is refused rather
/// than imported as the wrong mod.
fn pick_pending<'a>(
    pending: &'a [ManualDownload],
    path: &std::path::Path,
    project: Option<&str>,
) -> Option<&'a ManualDownload> {
    if let Some(project) = project {
        return pending.iter().find(|entry| entry.project_id == project);
    }
    let name = path.file_name()?.to_str()?;
    pending.iter().find(|entry| entry.file_name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pending download, for the picker tests.
    fn manual(project_id: &str, file_name: &str) -> ManualDownload {
        ManualDownload {
            source: SourceId::CurseForge,
            kind: ContentKind::Mod,
            project_id: project_id.to_string(),
            version_id: "v1".to_string(),
            file_name: file_name.to_string(),
            page_url: "https://example.invalid/p".to_string(),
            fingerprint: None,
            sha1: None,
            world: None,
        }
    }

    #[test]
    fn the_picker_matches_on_the_file_name() {
        let pending = vec![manual("a", "one.jar"), manual("b", "two.jar")];
        let picked = pick_pending(&pending, std::path::Path::new("/tmp/two.jar"), None);
        assert_eq!(picked.map(|p| p.project_id.as_str()), Some("b"));
    }

    #[test]
    fn the_project_flag_beats_the_file_name() {
        let pending = vec![manual("a", "one.jar"), manual("b", "two.jar")];
        let picked = pick_pending(&pending, std::path::Path::new("/tmp/two.jar"), Some("a"));
        assert_eq!(picked.map(|p| p.project_id.as_str()), Some("a"));
        assert!(pick_pending(&pending, std::path::Path::new("/tmp/two.jar"), Some("c")).is_none());
    }

    #[test]
    fn an_unmatched_file_name_picks_nothing() {
        let pending = vec![manual("a", "one.jar")];
        assert!(pick_pending(&pending, std::path::Path::new("/tmp/other.jar"), None).is_none());
    }

    #[test]
    fn a_bad_kind_is_an_error() {
        assert!(parse_kind("mod").is_ok());
        assert!(parse_kind("plugin").is_err());
    }

    #[test]
    fn merging_outcomes_keeps_every_list() {
        let first = AddOutcome {
            installed: vec![ContentEntry::default()],
            skipped: vec!["x".to_string()],
            manual: vec![manual("a", "one.jar")],
        };
        let merged = merge(&[first.clone(), first]);
        assert_eq!(merged.installed.len(), 2);
        assert_eq!(merged.skipped.len(), 2);
        assert_eq!(merged.manual.len(), 2);
    }
}
