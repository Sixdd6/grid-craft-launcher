//! Zip builders shared by the Forge-like loader tests. Test-only.

use std::io::Write;
use std::path::Path;

/// Writes a zip at `path` holding `entries` as `(name, bytes)` pairs.
pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    let file = std::fs::File::create(path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zip.start_file(*name, opts).expect("start entry");
        zip.write_all(bytes).expect("write entry");
    }
    zip.finish().expect("finish zip");
}

/// Writes a jar whose manifest declares `main` as its `Main-Class`.
pub fn write_jar_with_main(path: &Path, main: &str) {
    let manifest = format!("Manifest-Version: 1.0\r\nMain-Class: {main}\r\n\r\n");
    write_zip(path, &[("META-INF/MANIFEST.MF", manifest.as_bytes())]);
}
