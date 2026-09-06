//! Asset index parsing, object download specs, and the legacy virtual asset layout.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::download::{DownloadSpec, link_or_copy};
use crate::paths::Root;

/// Production base URL for asset objects.
pub const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";

/// A parsed asset index: every game asset keyed by its virtual path.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AssetIndex {
    /// Assets keyed by their path inside the game's asset tree.
    pub objects: BTreeMap<String, AssetObject>,
    /// Pre-1.7 indexes need the objects laid out by path under `assets/virtual/<id>`.
    #[serde(default, rename = "virtual")]
    pub virtual_: bool,
    /// Very old indexes need the objects laid out under `<game_dir>/resources`.
    #[serde(default)]
    pub map_to_resources: bool,
}

/// One asset object: content hash and size.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AssetObject {
    /// Lowercase hex sha1, which is also the object's file name.
    pub hash: String,
    /// Size in bytes.
    pub size: u64,
}

/// The two-character shard directory an object hash lives under.
fn shard(hash: &str) -> &str {
    if hash.len() >= 2 { &hash[..2] } else { hash }
}

/// Path of an asset object in the cache.
pub fn object_path(root: &Root, hash: &str) -> std::path::PathBuf {
    root.assets_dir()
        .join("objects")
        .join(shard(hash))
        .join(hash)
}

/// Errors laying out a legacy asset tree.
#[derive(Debug, thiserror::Error)]
pub enum LegacyError {
    /// An asset key in the index would escape the target directory.
    #[error(transparent)]
    Path(#[from] crate::paths::Error),
    /// Linking or copying an object into place failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Builds one [`DownloadSpec`] per asset object, fetching from `resources_base`.
pub fn asset_specs(index: &AssetIndex, root: &Root, resources_base: &str) -> Vec<DownloadSpec> {
    let base = resources_base.trim_end_matches('/');
    index
        .objects
        .iter()
        .map(|(path, object)| DownloadSpec {
            url: format!("{base}/{}/{}", shard(&object.hash), object.hash),
            sha1: Some(object.hash.clone()),
            size: Some(object.size),
            dest: object_path(root, &object.hash),
            label: format!("asset {path}"),
        })
        .collect()
}

/// Lays out a legacy index by path: under `assets/virtual/<index_id>`, or in the game's
/// `resources` directory. Does nothing for a modern index.
pub fn materialize_legacy(
    index: &AssetIndex,
    root: &Root,
    index_id: &str,
    game_dir: &Path,
) -> Result<(), LegacyError> {
    if !index.virtual_ && !index.map_to_resources {
        return Ok(());
    }
    let target = if index.map_to_resources {
        game_dir.join("resources")
    } else {
        root.assets_dir().join("virtual").join(index_id)
    };
    for (path, object) in &index.objects {
        // The key comes from Mojang's index; route it through `safe_join` so a `../` key
        // cannot write outside the virtual tree or the game directory.
        let dest = crate::paths::safe_join(&target, path)?;
        let src = object_path(root, &object.hash);
        if !src.is_file() {
            continue;
        }
        link_or_copy(&src, &dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = include_str!("../../../../tests/fixtures/mojang/asset_index_1.20.json");

    fn index() -> AssetIndex {
        serde_json::from_str(INDEX).expect("fixture parses")
    }

    #[test]
    fn asset_specs_cover_every_object() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let specs = asset_specs(&index(), &root, RESOURCES_BASE);
        assert_eq!(specs.len(), 20);
        let first = &specs[0];
        assert_eq!(first.label, "asset icons/icon_128x128.png");
        assert_eq!(
            first.url,
            "https://resources.download.minecraft.net/b6/b62ca8ec10d07e6bf5ac8dae0c8c1d2e6a1e3356"
        );
        assert_eq!(
            first.dest,
            root.assets_dir()
                .join("objects/b6/b62ca8ec10d07e6bf5ac8dae0c8c1d2e6a1e3356")
        );
        assert_eq!(first.size, Some(9101));
        assert_eq!(
            first.sha1.as_deref(),
            Some("b62ca8ec10d07e6bf5ac8dae0c8c1d2e6a1e3356")
        );
    }

    #[test]
    fn asset_specs_honour_a_custom_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let specs = asset_specs(&index(), &root, "http://127.0.0.1:1/res/");
        assert!(
            specs[0].url.starts_with("http://127.0.0.1:1/res/b6/"),
            "{}",
            specs[0].url
        );
    }

    #[test]
    fn a_modern_index_needs_no_legacy_layout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        materialize_legacy(&index(), &root, "1.20", dir.path()).expect("no-op");
        assert!(!root.assets_dir().join("virtual").exists());
    }

    #[test]
    fn a_virtual_index_is_laid_out_by_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mut index = AssetIndex {
            objects: BTreeMap::new(),
            virtual_: true,
            map_to_resources: false,
        };
        let hash = "0123456789abcdef0123456789abcdef01234567";
        index.objects.insert(
            "lang/en_GB.lang".to_string(),
            AssetObject {
                hash: hash.to_string(),
                size: 4,
            },
        );
        let src = object_path(&root, hash);
        std::fs::create_dir_all(src.parent().expect("parent")).expect("mkdir");
        std::fs::write(&src, b"hi\n!").expect("write object");

        materialize_legacy(&index, &root, "legacy", dir.path()).expect("materializes");
        let laid_out = root.assets_dir().join("virtual/legacy/lang/en_GB.lang");
        assert_eq!(std::fs::read(laid_out).expect("read"), b"hi\n!");
    }

    #[test]
    fn map_to_resources_writes_into_the_game_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let game = dir.path().join("instance");
        let hash = "89abcdef0123456789abcdef0123456789abcdef";
        let mut index = AssetIndex {
            objects: BTreeMap::new(),
            virtual_: false,
            map_to_resources: true,
        };
        index.objects.insert(
            "sound/step.ogg".to_string(),
            AssetObject {
                hash: hash.to_string(),
                size: 3,
            },
        );
        let src = object_path(&root, hash);
        std::fs::create_dir_all(src.parent().expect("parent")).expect("mkdir");
        std::fs::write(&src, b"ogg").expect("write object");

        materialize_legacy(&index, &root, "pre-1.6", &game).expect("materializes");
        assert!(game.join("resources/sound/step.ogg").is_file());
    }

    #[test]
    fn an_asset_key_that_escapes_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let mut index = AssetIndex {
            objects: BTreeMap::new(),
            virtual_: true,
            map_to_resources: false,
        };
        index.objects.insert(
            "../../escape.txt".to_string(),
            AssetObject {
                hash: hash.to_string(),
                size: 1,
            },
        );
        assert!(matches!(
            materialize_legacy(&index, &root, "legacy", dir.path()),
            Err(LegacyError::Path(crate::paths::Error::UnsafePath(_)))
        ));
        assert!(!dir.path().join("escape.txt").exists());
    }

    #[test]
    fn a_virtual_flag_parses_from_json() {
        let index: AssetIndex =
            serde_json::from_str(r#"{"objects":{},"virtual":true}"#).expect("parses");
        assert!(index.virtual_);
        assert!(!index.map_to_resources);
    }
}
