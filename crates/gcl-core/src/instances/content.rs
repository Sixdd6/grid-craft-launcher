//! Placing installed content into an instance's game directory.
//!
//! Every function here works on one [`Instance`]: it moves a file into the right folder under
//! `.minecraft/`, records a [`ContentEntry`] in `instance.toml`, and saves the config. The
//! target folder per kind is in the `mod-sources` skill; disabling renames a file to
//! `<file>.disabled`.

use std::path::{Path, PathBuf};

use super::model::{ContentEntry, ContentKind};
use super::{Error, Instance};
use crate::download::link_or_copy;
use crate::paths::safe_join;
use crate::sources::SourceId;

/// Suffix a disabled file or world folder carries on disk.
const DISABLED_SUFFIX: &str = ".disabled";

/// A file or world folder that was just installed, and the entry recorded for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// Where the file or world folder now lives.
    pub path: PathBuf,
    /// The entry as it was written to `instance.toml`.
    pub entry: ContentEntry,
}

/// The directory under `game_dir` that content of `kind` installs into.
///
/// `world` is required for [`ContentKind::DataPack`] and goes through [`safe_join`], so a world
/// name cannot escape `saves/`. The directory is not created here; the placing functions create
/// it on demand.
pub fn target_dir(
    game_dir: &Path,
    kind: ContentKind,
    world: Option<&str>,
) -> Result<PathBuf, Error> {
    let dir = match kind {
        ContentKind::Mod => game_dir.join("mods"),
        ContentKind::ResourcePack => game_dir.join("resourcepacks"),
        ContentKind::Shader => game_dir.join("shaderpacks"),
        ContentKind::World => game_dir.join("saves"),
        ContentKind::DataPack => {
            let world = world.ok_or(Error::WorldRequired)?;
            safe_join(&game_dir.join("saves"), world)?.join("datapacks")
        }
    };
    Ok(dir)
}

/// Where `entry`'s file lives right now, including the `.disabled` suffix when it is disabled.
///
/// For a [`ContentKind::World`] entry the "file" is the world folder `saves/<world>`.
pub fn file_path(instance: &Instance, entry: &ContentEntry) -> Result<PathBuf, Error> {
    let game_dir = instance.game_dir();
    let base = match entry.kind {
        ContentKind::World => {
            let world = entry.world.as_deref().ok_or(Error::WorldRequired)?;
            safe_join(&game_dir.join("saves"), world)?
        }
        kind => {
            let dir = target_dir(&game_dir, kind, entry.world.as_deref())?;
            safe_join(&dir, &entry.file_name)?
        }
    };
    Ok(if entry.enabled {
        base
    } else {
        with_disabled_suffix(&base)
    })
}

/// The entry installed from `source` with project id `project_id`, when there is one.
pub fn installed<'a>(
    instance: &'a Instance,
    source: SourceId,
    project_id: &str,
) -> Option<&'a ContentEntry> {
    let source = source.to_string();
    instance
        .config
        .content
        .iter()
        .find(|entry| entry.source == source && entry.project_id == project_id)
}

/// Links a cached object into its target folder and records `entry` in `instance.toml`.
///
/// `object` is a file under `cache/objects/`; it is hard-linked, or copied across filesystems.
/// An entry already installed from the same `(source, project_id)` is replaced where it sits in
/// the content list, and its file (or `.disabled` twin) is deleted first. The placed file is
/// always enabled.
#[tracing::instrument(skip(instance, object, entry), fields(slug = %instance.slug))]
pub fn place_file(
    instance: &mut Instance,
    object: &Path,
    entry: ContentEntry,
) -> Result<Placed, Error> {
    let mut entry = entry;
    entry.enabled = true;
    let dir = target_dir(&instance.game_dir(), entry.kind, entry.world.as_deref())?;
    let dest = safe_join(&dir, &entry.file_name)?;

    let existing = index_of(instance, &entry.source, &entry.project_id);
    if let Some(i) = existing {
        let old = instance.config.content[i].clone();
        remove_twins(instance, &old)?;
    }
    create_dir(&dir)?;
    link_or_copy(object, &dest).map_err(|source| Error::Io {
        path: dest.clone(),
        source,
    })?;
    record(instance, existing, entry.clone());
    instance.save()?;
    tracing::info!(path = %dest.display(), "placed content file");
    Ok(Placed { path: dest, entry })
}

/// Extracts a world zip under `saves/` and records `entry` in `instance.toml`.
///
/// The archive must hold exactly one top-level folder; that folder name becomes `entry.world`.
/// Every archive entry goes through [`safe_join`], so no entry can write outside `saves/`. An
/// existing `saves/<folder>` is never overwritten.
///
/// This blocks on zip and file I/O. Async callers wrap it in `tokio::task::spawn_blocking`.
#[tracing::instrument(skip(instance, entry), fields(slug = %instance.slug))]
pub fn place_world(
    instance: &mut Instance,
    zip_path: &Path,
    entry: ContentEntry,
) -> Result<Placed, Error> {
    let mut entry = entry;
    entry.kind = ContentKind::World;
    entry.enabled = true;
    let saves = instance.game_dir().join("saves");

    let file = std::fs::File::open(zip_path).map_err(io_at(zip_path))?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_at(zip_path))?;
    let (folder, plan) = plan_world(&mut archive, zip_path)?;

    let world_dir = safe_join(&saves, &folder)?;
    if world_dir.exists() {
        return Err(Error::WorldExists(folder));
    }
    let existing = index_of(instance, &entry.source, &entry.project_id);
    if let Some(i) = existing {
        let old = instance.config.content[i].clone();
        remove_twins(instance, &old)?;
    }

    for (index, rel) in plan {
        let dest = safe_join(&saves, &rel)?;
        let mut source = archive.by_index(index).map_err(zip_at(zip_path))?;
        if let Some(parent) = dest.parent() {
            create_dir(parent)?;
        }
        let mut out = std::fs::File::create(&dest).map_err(io_at(&dest))?;
        std::io::copy(&mut source, &mut out).map_err(io_at(&dest))?;
    }

    entry.world = Some(folder);
    record(instance, existing, entry.clone());
    instance.save()?;
    tracing::info!(path = %world_dir.display(), "extracted world");
    Ok(Placed {
        path: world_dir,
        entry,
    })
}

/// Enables or disables the content with `project_id` by renaming its file.
///
/// Returns the path the file has afterwards. An entry already in the wanted state is left alone.
pub fn set_enabled(
    instance: &mut Instance,
    project_id: &str,
    enabled: bool,
) -> Result<PathBuf, Error> {
    let i = index_of_project(instance, project_id)?;
    let current = file_path(instance, &instance.config.content[i])?;
    if instance.config.content[i].enabled == enabled {
        return Ok(current);
    }
    let wanted = file_path(
        instance,
        &ContentEntry {
            enabled,
            ..instance.config.content[i].clone()
        },
    )?;
    if std::fs::symlink_metadata(&current).is_err() {
        return Err(Error::ContentFileMissing(current));
    }
    std::fs::rename(&current, &wanted).map_err(io_at(&wanted))?;
    instance.config.content[i].enabled = enabled;
    instance.save()?;
    Ok(wanted)
}

/// Deletes the content with `project_id` from disk and from `instance.toml`.
///
/// A file that is already gone is not an error; an unknown project id is [`Error::ContentNotFound`].
pub fn remove(instance: &mut Instance, project_id: &str) -> Result<(), Error> {
    let i = index_of_project(instance, project_id)?;
    let entry = instance.config.content[i].clone();
    remove_twins(instance, &entry)?;
    instance.config.content.remove(i);
    instance.save()
}

/// Replaces the entry at `existing`, or appends `entry` when there is none.
fn record(instance: &mut Instance, existing: Option<usize>, entry: ContentEntry) {
    match existing {
        Some(i) => instance.config.content[i] = entry,
        None => instance.config.content.push(entry),
    }
}

/// Index of the entry installed from `(source, project_id)`.
fn index_of(instance: &Instance, source: &str, project_id: &str) -> Option<usize> {
    instance
        .config
        .content
        .iter()
        .position(|entry| entry.source == source && entry.project_id == project_id)
}

/// Index of the entry with `project_id`, or [`Error::ContentNotFound`].
fn index_of_project(instance: &Instance, project_id: &str) -> Result<usize, Error> {
    instance
        .config
        .content
        .iter()
        .position(|entry| entry.project_id == project_id)
        .ok_or_else(|| Error::ContentNotFound(project_id.to_string()))
}

/// Deletes both the enabled and the disabled form of `entry`'s file. Missing files are fine.
fn remove_twins(instance: &Instance, entry: &ContentEntry) -> Result<(), Error> {
    let base = file_path(
        instance,
        &ContentEntry {
            enabled: true,
            ..entry.clone()
        },
    )?;
    remove_path(&base)?;
    remove_path(&with_disabled_suffix(&base))
}

/// Deletes a file or a whole directory. A path that is not there is not an error.
fn remove_path(path: &Path) -> Result<(), Error> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if meta.is_dir() {
        std::fs::remove_dir_all(path).map_err(io_at(path))
    } else {
        std::fs::remove_file(path).map_err(io_at(path))
    }
}

/// Reads the archive's file entries: the single top-level folder, and every entry to extract.
fn plan_world<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    zip_path: &Path,
) -> Result<(String, Vec<(usize, String)>), Error> {
    let mut folder: Option<String> = None;
    let mut plan = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(zip_at(zip_path))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            return Err(Error::BadWorldZip(format!(
                "{} would escape the world folder",
                entry.name()
            )));
        };
        let parts: Vec<String> = name
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(part) => {
                    Some(part.to_string_lossy().replace('\\', "/"))
                }
                _ => None,
            })
            .collect();
        if parts.len() < 2 {
            return Err(Error::BadWorldZip(format!(
                "{} is not inside a world folder",
                entry.name()
            )));
        }
        let top = parts[0].clone();
        match &folder {
            Some(seen) if *seen != top => {
                return Err(Error::BadWorldZip(format!(
                    "more than one top-level folder: {seen} and {top}"
                )));
            }
            _ => folder = Some(top),
        }
        plan.push((index, parts.join("/")));
    }
    let folder =
        folder.ok_or_else(|| Error::BadWorldZip("the archive holds no files".to_string()))?;
    Ok((folder, plan))
}

/// The same path with `.disabled` appended to its last component.
fn with_disabled_suffix(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(DISABLED_SUFFIX);
    PathBuf::from(name)
}

fn create_dir(path: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(path).map_err(io_at(path))
}

/// Builds an [`Error::Io`] mapper that carries `path`.
fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io { path, source }
}

/// Builds an [`Error::Zip`] mapper that carries `path`.
fn zip_at(path: &Path) -> impl Fn(zip::result::ZipError) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Zip {
        path: path.clone(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{ContentEntry, ContentKind, Loader};
    use super::super::{Error, Instance, Instances};
    use super::*;
    use crate::paths::Root;
    use crate::sources::SourceId;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A tempdir root with one instance in it.
    fn fixture() -> (tempfile::TempDir, Instance) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let instance = Instances::new(root)
            .create("Test", "1.20.1", Loader::Fabric, None, &BTreeMap::new())
            .expect("create");
        (dir, instance)
    }

    /// Writes `bytes` to a file under the tempdir, standing in for a cached object.
    fn object(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join("cache").join("objects").join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, bytes).expect("write object");
        path
    }

    /// Writes a zip at `path` holding `entries` as `(name, bytes)` pairs.
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zip.start_file(*name, opts).expect("start entry");
            zip.write_all(bytes).expect("write entry");
        }
        zip.finish().expect("finish zip");
    }

    fn mod_entry(project_id: &str, file_name: &str) -> ContentEntry {
        ContentEntry {
            source: "modrinth".to_string(),
            project_id: project_id.to_string(),
            version_id: "v1".to_string(),
            file_name: file_name.to_string(),
            kind: ContentKind::Mod,
            ..ContentEntry::default()
        }
    }

    #[test]
    fn target_dir_maps_every_kind_to_its_folder() {
        let game = Path::new("/game");
        assert_eq!(
            target_dir(game, ContentKind::Mod, None).expect("mod"),
            game.join("mods")
        );
        assert_eq!(
            target_dir(game, ContentKind::ResourcePack, None).expect("resourcepack"),
            game.join("resourcepacks")
        );
        assert_eq!(
            target_dir(game, ContentKind::Shader, None).expect("shader"),
            game.join("shaderpacks")
        );
        assert_eq!(
            target_dir(game, ContentKind::World, None).expect("world"),
            game.join("saves")
        );
        assert_eq!(
            target_dir(game, ContentKind::DataPack, Some("My World")).expect("datapack"),
            game.join("saves").join("My World").join("datapacks")
        );
    }

    #[test]
    fn a_datapack_without_a_world_is_world_required() {
        let game = Path::new("/game");
        assert!(matches!(
            target_dir(game, ContentKind::DataPack, None),
            Err(Error::WorldRequired)
        ));
    }

    #[test]
    fn a_datapack_world_that_escapes_saves_is_rejected() {
        let game = Path::new("/game");
        for bad in ["../evil", "/etc", ".."] {
            assert!(
                matches!(
                    target_dir(game, ContentKind::DataPack, Some(bad)),
                    Err(Error::Paths(_))
                ),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn place_file_links_the_object_into_mods_and_records_the_entry() {
        let (dir, mut instance) = fixture();
        let src = object(&dir, "aaa", b"jar bytes");
        let placed =
            place_file(&mut instance, &src, mod_entry("AANobbMI", "sodium.jar")).expect("place");

        assert_eq!(
            placed.path,
            instance.game_dir().join("mods").join("sodium.jar")
        );
        assert_eq!(std::fs::read(&placed.path).expect("read"), b"jar bytes");
        assert_eq!(instance.config.content.len(), 1);
        assert_eq!(instance.config.content[0].file_name, "sodium.jar");

        // The entry survived the save to instance.toml.
        let text = std::fs::read_to_string(instance.config_path()).expect("read toml");
        assert!(text.contains("sodium.jar"), "{text}");
    }

    #[test]
    fn placing_the_same_project_again_replaces_the_old_file_in_place() {
        let (dir, mut instance) = fixture();
        let first = object(&dir, "aaa", b"old");
        let second = object(&dir, "bbb", b"new");
        place_file(&mut instance, &first, mod_entry("AANobbMI", "sodium-1.jar")).expect("first");
        place_file(
            &mut instance,
            &object(&dir, "ccc", b"other"),
            mod_entry("other", "lithium.jar"),
        )
        .expect("other");
        let placed = place_file(
            &mut instance,
            &second,
            ContentEntry {
                version_id: "v2".to_string(),
                ..mod_entry("AANobbMI", "sodium-2.jar")
            },
        )
        .expect("second");

        let mods = instance.game_dir().join("mods");
        assert!(!mods.join("sodium-1.jar").exists(), "the old file went");
        assert_eq!(std::fs::read(&placed.path).expect("read"), b"new");
        // Replaced in place: two entries, the updated one still first.
        assert_eq!(instance.config.content.len(), 2);
        assert_eq!(instance.config.content[0].file_name, "sodium-2.jar");
        assert_eq!(instance.config.content[0].version_id, "v2");
        assert_eq!(instance.config.content[1].project_id, "other");
    }

    #[test]
    fn replacing_a_disabled_entry_removes_its_disabled_twin() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"old"),
            mod_entry("AANobbMI", "sodium-1.jar"),
        )
        .expect("first");
        set_enabled(&mut instance, "AANobbMI", false).expect("disable");
        let mods = instance.game_dir().join("mods");
        assert!(mods.join("sodium-1.jar.disabled").is_file());

        place_file(
            &mut instance,
            &object(&dir, "bbb", b"new"),
            mod_entry("AANobbMI", "sodium-2.jar"),
        )
        .expect("second");
        assert!(!mods.join("sodium-1.jar.disabled").exists());
        assert!(mods.join("sodium-2.jar").is_file());
    }

    #[test]
    fn set_enabled_false_renames_to_disabled_and_installed_shows_it() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"jar"),
            mod_entry("AANobbMI", "sodium.jar"),
        )
        .expect("place");

        let path = set_enabled(&mut instance, "AANobbMI", false).expect("disable");
        let mods = instance.game_dir().join("mods");
        assert_eq!(path, mods.join("sodium.jar.disabled"));
        assert!(path.is_file());
        assert!(!mods.join("sodium.jar").exists());

        let entry = installed(&instance, SourceId::Modrinth, "AANobbMI").expect("installed");
        assert!(!entry.enabled);
        assert_eq!(file_path(&instance, entry).expect("path"), path);

        // Disabling twice is a no-op that reports the same path.
        assert_eq!(
            set_enabled(&mut instance, "AANobbMI", false).expect("again"),
            path
        );

        let back = set_enabled(&mut instance, "AANobbMI", true).expect("enable");
        assert_eq!(back, mods.join("sodium.jar"));
        assert!(back.is_file());
        assert!(
            installed(&instance, SourceId::Modrinth, "AANobbMI")
                .expect("entry")
                .enabled
        );
    }

    #[test]
    fn set_enabled_with_the_file_gone_is_content_file_missing() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"jar"),
            mod_entry("AANobbMI", "sodium.jar"),
        )
        .expect("place");
        std::fs::remove_file(instance.game_dir().join("mods").join("sodium.jar")).expect("rm");

        assert!(matches!(
            set_enabled(&mut instance, "AANobbMI", false),
            Err(Error::ContentFileMissing(_))
        ));
    }

    #[test]
    fn set_enabled_for_an_unknown_project_is_content_not_found() {
        let (_dir, mut instance) = fixture();
        assert!(matches!(
            set_enabled(&mut instance, "nope", false),
            Err(Error::ContentNotFound(id)) if id == "nope"
        ));
    }

    #[test]
    fn remove_deletes_the_file_and_the_entry() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"jar"),
            mod_entry("AANobbMI", "sodium.jar"),
        )
        .expect("place");
        let path = instance.game_dir().join("mods").join("sodium.jar");

        remove(&mut instance, "AANobbMI").expect("remove");
        assert!(!path.exists());
        assert!(instance.config.content.is_empty());
        assert!(installed(&instance, SourceId::Modrinth, "AANobbMI").is_none());
        assert!(matches!(
            remove(&mut instance, "AANobbMI"),
            Err(Error::ContentNotFound(_))
        ));
    }

    #[test]
    fn remove_deletes_a_disabled_twin_and_tolerates_a_missing_file() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"jar"),
            mod_entry("AANobbMI", "sodium.jar"),
        )
        .expect("place");
        set_enabled(&mut instance, "AANobbMI", false).expect("disable");
        remove(&mut instance, "AANobbMI").expect("remove disabled");
        assert!(
            !instance
                .game_dir()
                .join("mods")
                .join("sodium.jar.disabled")
                .exists()
        );

        place_file(
            &mut instance,
            &object(&dir, "bbb", b"jar"),
            mod_entry("second", "lithium.jar"),
        )
        .expect("place");
        std::fs::remove_file(instance.game_dir().join("mods").join("lithium.jar")).expect("rm");
        remove(&mut instance, "second").expect("remove with no file");
        assert!(instance.config.content.is_empty());
    }

    #[test]
    fn installed_matches_on_source_and_project_id() {
        let (dir, mut instance) = fixture();
        place_file(
            &mut instance,
            &object(&dir, "aaa", b"jar"),
            mod_entry("AANobbMI", "sodium.jar"),
        )
        .expect("place");
        assert!(installed(&instance, SourceId::Modrinth, "AANobbMI").is_some());
        assert!(installed(&instance, SourceId::CurseForge, "AANobbMI").is_none());
        assert!(installed(&instance, SourceId::Modrinth, "other").is_none());
    }

    #[test]
    fn place_world_extracts_the_zip_under_saves() {
        let (dir, mut instance) = fixture();
        let zip_path = dir.path().join("world.zip");
        write_zip(
            &zip_path,
            &[
                ("MyWorld/level.dat", b"level".as_slice()),
                ("MyWorld/region/r.0.0.mca", b"region".as_slice()),
            ],
        );
        let entry = ContentEntry {
            kind: ContentKind::World,
            file_name: "world.zip".to_string(),
            ..mod_entry("world-1", "world.zip")
        };

        let placed = place_world(&mut instance, &zip_path, entry).expect("place world");
        let saves = instance.game_dir().join("saves");
        assert_eq!(placed.path, saves.join("MyWorld"));
        assert_eq!(
            std::fs::read(saves.join("MyWorld").join("level.dat")).expect("read"),
            b"level"
        );
        assert!(
            saves
                .join("MyWorld")
                .join("region")
                .join("r.0.0.mca")
                .is_file()
        );
        assert_eq!(placed.entry.world.as_deref(), Some("MyWorld"));
        assert_eq!(instance.config.content[0].world.as_deref(), Some("MyWorld"));
        assert_eq!(
            file_path(&instance, &instance.config.content[0]).expect("path"),
            saves.join("MyWorld")
        );
    }

    #[test]
    fn place_world_rejects_an_entry_that_escapes_saves() {
        let (dir, mut instance) = fixture();
        let zip_path = dir.path().join("evil.zip");
        write_zip(&zip_path, &[("../x", b"escape".as_slice())]);
        let entry = ContentEntry {
            kind: ContentKind::World,
            ..mod_entry("evil", "evil.zip")
        };

        assert!(matches!(
            place_world(&mut instance, &zip_path, entry),
            Err(Error::BadWorldZip(_))
        ));
        assert!(!dir.path().join("x").exists());
        assert!(instance.config.content.is_empty());
    }

    #[test]
    fn place_world_rejects_a_zip_with_more_than_one_top_folder() {
        let (dir, mut instance) = fixture();
        let zip_path = dir.path().join("two.zip");
        write_zip(
            &zip_path,
            &[
                ("A/level.dat", b"a".as_slice()),
                ("B/level.dat", b"b".as_slice()),
            ],
        );
        let entry = ContentEntry {
            kind: ContentKind::World,
            ..mod_entry("two", "two.zip")
        };
        assert!(matches!(
            place_world(&mut instance, &zip_path, entry),
            Err(Error::BadWorldZip(_))
        ));
    }

    #[test]
    fn place_world_rejects_a_zip_with_a_file_at_its_root() {
        let (dir, mut instance) = fixture();
        let zip_path = dir.path().join("flat.zip");
        write_zip(&zip_path, &[("level.dat", b"a".as_slice())]);
        let entry = ContentEntry {
            kind: ContentKind::World,
            ..mod_entry("flat", "flat.zip")
        };
        assert!(matches!(
            place_world(&mut instance, &zip_path, entry),
            Err(Error::BadWorldZip(_))
        ));
    }

    #[test]
    fn place_world_refuses_to_overwrite_an_existing_world() {
        let (dir, mut instance) = fixture();
        let zip_path = dir.path().join("world.zip");
        write_zip(&zip_path, &[("MyWorld/level.dat", b"level".as_slice())]);
        let entry = ContentEntry {
            kind: ContentKind::World,
            ..mod_entry("world-1", "world.zip")
        };
        place_world(&mut instance, &zip_path, entry.clone()).expect("first");

        assert!(matches!(
            place_world(&mut instance, &zip_path, entry),
            Err(Error::WorldExists(name)) if name == "MyWorld"
        ));
    }

    #[test]
    fn place_file_puts_a_datapack_in_its_world_folder() {
        let (dir, mut instance) = fixture();
        let entry = ContentEntry {
            kind: ContentKind::DataPack,
            world: Some("MyWorld".to_string()),
            ..mod_entry("pack-1", "pack.zip")
        };
        let placed =
            place_file(&mut instance, &object(&dir, "aaa", b"pack"), entry).expect("place");
        assert_eq!(
            placed.path,
            instance
                .game_dir()
                .join("saves")
                .join("MyWorld")
                .join("datapacks")
                .join("pack.zip")
        );
        assert!(placed.path.is_file());
    }

    #[test]
    fn place_file_rejects_a_file_name_that_escapes_its_folder() {
        let (dir, mut instance) = fixture();
        let src = object(&dir, "aaa", b"jar");
        assert!(matches!(
            place_file(&mut instance, &src, mod_entry("bad", "../evil.jar")),
            Err(Error::Paths(_))
        ));
        assert!(instance.config.content.is_empty());
    }
}
