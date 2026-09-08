//! Instance directories and the `instance.toml` file that describes each one.
//!
//! One instance is one directory under `<root>/instances/<slug>/`, holding `instance.toml`
//! and a `.minecraft/` game directory. See the `instance-model` skill for the schema.

pub mod content;
pub mod model;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::paths::{Root, slugify, unique_slug, write_atomic};
use model::{InstanceConfig, Loader};

/// File name of the per-instance config inside an instance directory.
const CONFIG_FILE: &str = "instance.toml";

/// Directories created inside `.minecraft/` for every new instance. See SPEC R2.3.
const GAME_SUBDIRS: [&str; 5] = ["mods", "resourcepacks", "shaderpacks", "saves", "logs"];

/// Errors reading, writing, or resolving instances.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No instance exists with the given slug.
    #[error("no instance named {0}")]
    NotFound(String),
    /// An instance directory already exists where a new one was to be created.
    #[error("an instance already exists at {0}")]
    AlreadyExists(String),
    /// An I/O operation on an instance path failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// An `instance.toml` could not be parsed.
    #[error("could not parse instance at {path}: {source}")]
    Parse {
        /// The file being parsed.
        path: PathBuf,
        /// The underlying parse error.
        source: toml::de::Error,
    },
    /// An `instance.toml` could not be serialized.
    #[error(transparent)]
    Serialize(#[from] toml::ser::Error),
    /// The `options.txt` preseed could not be written.
    #[error(transparent)]
    Settings(#[from] crate::settings::Error),
    /// A data pack was installed without naming the world it belongs to.
    #[error("this content kind needs a target world")]
    WorldRequired,
    /// No content entry in this instance has the given project id.
    #[error("no installed content with project id {0}")]
    ContentNotFound(String),
    /// A content entry is recorded in `instance.toml` but its file is gone from disk.
    #[error("content file missing at {0}")]
    ContentFileMissing(PathBuf),
    /// A world folder of that name already exists under `saves/`.
    #[error("a world named {0} already exists")]
    WorldExists(String),
    /// A world zip is not one folder holding the world.
    #[error("bad world zip: {0}")]
    BadWorldZip(String),
    /// A path under the app root could not be built or was unsafe.
    #[error(transparent)]
    Paths(#[from] crate::paths::Error),
    /// A zip archive could not be read.
    #[error("could not read zip at {path}: {source}")]
    Zip {
        /// The archive being read.
        path: PathBuf,
        /// The underlying zip error.
        source: zip::result::ZipError,
    },
}

/// One instance on disk: its slug, its directory, and its parsed config.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Directory name under `instances/`, unique within the root.
    pub slug: String,
    /// Absolute path to the instance directory.
    pub dir: PathBuf,
    /// Parsed contents of `instance.toml`.
    pub config: InstanceConfig,
}

impl Instance {
    /// The game directory Minecraft runs in, `<dir>/.minecraft`.
    pub fn game_dir(&self) -> PathBuf {
        self.dir.join(".minecraft")
    }

    /// Path to this instance's `instance.toml`.
    pub fn config_path(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    /// Writes `instance.toml` back to disk.
    pub fn save(&self) -> Result<(), Error> {
        let path = self.config_path();
        let text = toml::to_string_pretty(&self.config)?;
        write_file(&path, text.as_bytes())
    }
}

/// Reads and writes the instances under one app root.
#[derive(Debug, Clone)]
pub struct Instances {
    root: Root,
    gc_default: model::GcPreset,
}

impl Instances {
    /// Builds an instance store over the given app root.
    pub fn new(root: Root) -> Self {
        Instances {
            root,
            gc_default: model::GcPreset::default(),
        }
    }

    /// Sets the garbage collector preset [`Instances::create`] seeds into a new instance.
    pub fn with_gc_default(mut self, gc: model::GcPreset) -> Self {
        self.gc_default = gc;
        self
    }

    /// The app root these instances live under.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// Creates an instance directory, its `instance.toml`, and its `.minecraft` folder.
    ///
    /// The slug comes from `name` and gains a `-2`, `-3` suffix on collision.
    /// `game_defaults` is written to `options.txt` as `key:value` lines when non-empty.
    #[tracing::instrument(skip(self, game_defaults))]
    pub fn create(
        &self,
        name: &str,
        minecraft: &str,
        loader: Loader,
        loader_version: Option<String>,
        game_defaults: &BTreeMap<String, String>,
    ) -> Result<Instance, Error> {
        let base = slugify(name);
        let slug = unique_slug(&base, |candidate| {
            self.root.instance_dir(candidate).exists()
        });
        let dir = self.root.instance_dir(&slug);
        if dir.exists() {
            return Err(Error::AlreadyExists(slug));
        }
        let config = InstanceConfig {
            name: name.to_string(),
            minecraft: minecraft.to_string(),
            loader,
            loader_version,
            created: now_rfc3339(),
            jvm: model::InstanceJvm {
                gc: self.gc_default,
                ..model::InstanceJvm::default()
            },
            ..InstanceConfig::default()
        };
        let instance = Instance {
            slug,
            dir: dir.clone(),
            config,
        };
        create_dir(&instance.game_dir())?;
        for sub in GAME_SUBDIRS {
            create_dir(&instance.game_dir().join(sub))?;
        }
        instance.save()?;
        crate::settings::apply_preseed(&instance.game_dir(), game_defaults)?;
        tracing::info!(slug = %instance.slug, "created instance");
        Ok(instance)
    }

    /// Lists every instance under the root, sorted by display name.
    ///
    /// Directories without an `instance.toml` are skipped with a warning.
    pub fn list(&self) -> Result<Vec<Instance>, Error> {
        let dir = self.root.instances_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(Error::Io { path: dir, source }),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !path.join(CONFIG_FILE).is_file() {
                tracing::warn!(path = %path.display(), "skipping directory with no instance.toml");
                continue;
            }
            let Some(slug) = path.file_name().and_then(|n| n.to_str()) else {
                tracing::warn!(path = %path.display(), "skipping directory with a non-UTF-8 name");
                continue;
            };
            match self.load(slug, path.clone()) {
                Ok(instance) => out.push(instance),
                Err(source) => {
                    tracing::warn!(path = %path.display(), %source, "skipping unreadable instance");
                }
            }
        }
        out.sort_by(|a, b| a.config.name.cmp(&b.config.name).then(a.slug.cmp(&b.slug)));
        Ok(out)
    }

    /// Loads one instance by slug.
    pub fn get(&self, slug: &str) -> Result<Instance, Error> {
        validate_slug(slug)?;
        let dir = self.root.instance_dir(slug);
        if !dir.join(CONFIG_FILE).is_file() {
            return Err(Error::NotFound(slug.to_string()));
        }
        self.load(slug, dir)
    }

    /// Changes an instance's display name. The slug and directory stay as they are.
    pub fn rename(&self, slug: &str, new_name: &str) -> Result<Instance, Error> {
        validate_slug(slug)?;
        let mut instance = self.get(slug)?;
        instance.config.name = new_name.to_string();
        instance.save()?;
        Ok(instance)
    }

    /// Deletes an instance directory and everything in it.
    pub fn delete(&self, slug: &str) -> Result<(), Error> {
        validate_slug(slug)?;
        let dir = self.root.instance_dir(slug);
        if !dir.is_dir() {
            return Err(Error::NotFound(slug.to_string()));
        }
        std::fs::remove_dir_all(&dir).map_err(|source| Error::Io { path: dir, source })?;
        tracing::info!(slug, "deleted instance");
        Ok(())
    }

    fn load(&self, slug: &str, dir: PathBuf) -> Result<Instance, Error> {
        let path = dir.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let config = toml::from_str(&text).map_err(|source| Error::Parse {
            path: path.clone(),
            source,
        })?;
        Ok(Instance {
            slug: slug.to_string(),
            dir,
            config,
        })
    }
}

/// Rejects a slug that is not a plain directory name this module could have created.
///
/// `get`, `rename`, and `delete` take the slug from the user, so a value like `../../x` would
/// otherwise name a directory outside the app root. A valid slug is non-empty, is already its
/// own [`slugify`] output, and holds no path separator or `..`.
fn validate_slug(slug: &str) -> Result<(), Error> {
    let ok = !slug.is_empty()
        && slug == slugify(slug)
        && !slug.contains('/')
        && !slug.contains('\\')
        && !slug.contains("..");
    if ok {
        Ok(())
    } else {
        Err(Error::NotFound(slug.to_string()))
    }
}

/// The current time as an RFC 3339 timestamp in UTC.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn create_dir(path: &std::path::Path) -> Result<(), Error> {
    std::fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes a file through a temp file in the same directory, then renames it into place.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), Error> {
    write_atomic(path, bytes).map_err(|err| match err {
        crate::paths::Error::Io { path, source } => Error::Io { path, source },
        other => Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(other.to_string()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Root;
    use model::Loader;
    use std::collections::BTreeMap;

    fn defaults(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn fixture() -> (tempfile::TempDir, Instances) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let instances = Instances::new(root);
        (dir, instances)
    }

    #[test]
    fn create_writes_instance_toml_that_round_trips() {
        let (_dir, instances) = fixture();
        let created = instances
            .create("My Pack", "1.20.1", Loader::Fabric, None, &BTreeMap::new())
            .expect("create");
        assert_eq!(created.slug, "my-pack");
        for sub in GAME_SUBDIRS {
            let path = created.game_dir().join(sub);
            assert!(path.is_dir(), "{path:?} was not created");
        }
        let text = std::fs::read_to_string(created.config_path()).expect("read");
        let parsed: model::InstanceConfig = toml::from_str(&text).expect("parse");
        assert_eq!(parsed, created.config);
        insta::with_settings!({filters => vec![
            (r"created = \x22[^\x22]*\x22", "created = \"[created]\""),
        ]}, {
            insta::assert_snapshot!("instance_toml", text);
        });
    }

    #[test]
    fn duplicate_name_gets_a_numbered_slug() {
        let (_dir, instances) = fixture();
        let first = instances
            .create("My Pack", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("first");
        let second = instances
            .create("My Pack", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("second");
        assert_eq!(first.slug, "my-pack");
        assert_eq!(second.slug, "my-pack-2");
        assert_eq!(second.config.name, "My Pack");
    }

    #[test]
    fn game_defaults_are_written_as_colon_separated_options() {
        let (_dir, instances) = fixture();
        let created = instances
            .create(
                "Opts",
                "1.20.1",
                Loader::None,
                None,
                &defaults(&[("renderDistance", "12")]),
            )
            .expect("create");
        let options =
            std::fs::read_to_string(created.game_dir().join("options.txt")).expect("read");
        assert_eq!(options, "renderDistance:12\n");
    }

    #[test]
    fn empty_game_defaults_write_no_options_file() {
        let (_dir, instances) = fixture();
        let created = instances
            .create("Bare", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        assert!(!created.game_dir().join("options.txt").exists());
    }

    #[test]
    fn list_returns_every_instance_sorted_by_name() {
        let (_dir, instances) = fixture();
        instances
            .create("Zebra", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("zebra");
        instances
            .create("Alpha", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("alpha");
        let names: Vec<String> = instances
            .list()
            .expect("list")
            .into_iter()
            .map(|i| i.config.name)
            .collect();
        assert_eq!(names, vec!["Alpha".to_string(), "Zebra".to_string()]);
    }

    #[test]
    fn list_skips_directories_without_instance_toml() {
        let (_dir, instances) = fixture();
        instances
            .create("Real", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("real");
        std::fs::create_dir_all(instances.root().instance_dir("junk")).expect("junk dir");
        let listed = instances.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "real");
    }

    #[test]
    fn list_skips_an_unparsable_instance_toml() {
        let (_dir, instances) = fixture();
        instances
            .create("Good", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("good");
        let junk = instances.root().instance_dir("broken");
        std::fs::create_dir_all(&junk).expect("mkdir");
        std::fs::write(junk.join("instance.toml"), "this is not = = toml").expect("write");
        let listed = instances.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "good");
    }

    #[test]
    fn save_leaves_no_temp_file_in_the_instance_directory() {
        let (_dir, instances) = fixture();
        let mut created = instances
            .create("Atomic", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        created.config.name = "Atomic Two".to_string();
        created.save().expect("save");
        let leftovers: Vec<String> = std::fs::read_dir(&created.dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert_eq!(
            instances.get("atomic").expect("get").config.name,
            "Atomic Two"
        );
    }

    #[test]
    fn rename_changes_the_name_and_keeps_the_slug() {
        let (_dir, instances) = fixture();
        let created = instances
            .create("Old Name", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        let renamed = instances.rename(&created.slug, "New Name").expect("rename");
        assert_eq!(renamed.slug, "old-name");
        assert_eq!(renamed.config.name, "New Name");
        let reloaded = instances.get("old-name").expect("get");
        assert_eq!(reloaded.config.name, "New Name");
    }

    #[test]
    fn delete_removes_the_directory_and_get_then_fails() {
        let (_dir, instances) = fixture();
        let created = instances
            .create("Gone", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        instances.delete(&created.slug).expect("delete");
        assert!(!created.dir.exists());
        assert!(matches!(
            instances.get("gone"),
            Err(Error::NotFound(slug)) if slug == "gone"
        ));
    }

    #[test]
    fn delete_of_a_missing_slug_is_not_found() {
        let (_dir, instances) = fixture();
        assert!(matches!(instances.delete("nope"), Err(Error::NotFound(_))));
    }

    #[test]
    fn delete_refuses_a_slug_that_escapes_the_instances_directory() {
        let (dir, instances) = fixture();
        let sibling = dir.path().join("keep-me");
        std::fs::create_dir_all(&sibling).expect("sibling");
        std::fs::write(sibling.join("data.txt"), b"precious").expect("write");

        // `instances/../keep-me` is the sibling directory, one level above the instances root.
        assert!(matches!(
            instances.delete("../keep-me"),
            Err(Error::NotFound(slug)) if slug == "../keep-me"
        ));
        assert!(sibling.join("data.txt").is_file(), "the sibling survived");
        assert!(matches!(instances.delete("../x"), Err(Error::NotFound(_))));
    }

    #[test]
    fn get_refuses_a_slug_with_a_path_separator() {
        let (_dir, instances) = fixture();
        for bad in ["a/b", "a\\b", "..", "", "../x", "Upper"] {
            assert!(
                matches!(instances.get(bad), Err(Error::NotFound(_))),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn rename_refuses_a_slug_that_escapes() {
        let (_dir, instances) = fixture();
        assert!(matches!(
            instances.rename("../x", "New"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn get_of_a_missing_slug_is_not_found() {
        let (_dir, instances) = fixture();
        assert!(matches!(instances.get("nope"), Err(Error::NotFound(_))));
    }

    #[test]
    fn create_seeds_the_gc_preset_from_the_config_default() {
        let (_dir, instances) = fixture();
        let seeded = instances
            .clone()
            .with_gc_default(model::GcPreset::Zgc)
            .create("Zed", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        assert_eq!(seeded.config.jvm.gc, model::GcPreset::Zgc);
        let text = std::fs::read_to_string(seeded.config_path()).expect("read");
        assert!(text.contains("gc = \"zgc\""), "{text}");

        let plain = instances
            .create("Plain", "1.20.1", Loader::None, None, &BTreeMap::new())
            .expect("create");
        assert_eq!(plain.config.jvm.gc, model::GcPreset::Default);
        let text = std::fs::read_to_string(plain.config_path()).expect("read");
        assert!(!text.contains("gc ="), "{text}");
    }
}
