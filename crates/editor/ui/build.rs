use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("az-editor-ui crate should live under crates/editor/ui");
    let source_dir = repo_root.join("resources").join("themes");

    println!("cargo:rerun-if-changed={}", source_dir.display());

    if let Err(error) = generate_bundled_theme_sources(&source_dir) {
        panic!("failed to generate bundled theme sources: {error}");
    }
}

fn generate_bundled_theme_sources(source_dir: &Path) -> io::Result<()> {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let output_path = out_dir.join("bundled_themes.rs");
    let mut entries = Vec::new();

    if source_dir.is_dir() {
        for entry in fs::read_dir(source_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            entries.push((entry.file_name().to_string_lossy().into_owned(), path));
        }
    }

    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated = String::from("pub const BUNDLED_THEMES: &[(&str, &str)] = &[\n");
    for (file_name, path) in entries {
        generated.push_str("    (");
        let _ = write!(generated, "{file_name:?}");
        generated.push_str(", include_str!(r#\"");
        generated.push_str(&path.to_string_lossy());
        generated.push_str("\"#)),\n");
    }
    generated.push_str("];\n");

    if fs::read_to_string(&output_path).ok().as_deref() == Some(generated.as_str()) {
        return Ok(());
    }

    fs::write(output_path, generated)
}
