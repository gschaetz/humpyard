//! Embeds the built admin GUI when the `ui` feature is on (ADR 0032).
//!
//! With the feature off it writes an empty file list, so a plain `cargo build` needs no Node. With it on it
//! fails the build unless `ui/dist/index.html` exists, so a release can never carry the placeholder by accident.
//! `HUMPYARD_UI_DIST` names another directory (the build-failure test uses it).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

fn main() -> ExitCode {
    match generate() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn generate() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=HUMPYARD_UI_DIST");
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let dist =
        env::var_os("HUMPYARD_UI_DIST").map_or_else(|| manifest.join("ui/dist"), PathBuf::from);
    println!("cargo:rerun-if-changed={}", dist.display());

    let mut files = Vec::new();
    if env::var_os("CARGO_FEATURE_UI").is_some() {
        if !dist.join("index.html").is_file() {
            return Err(format!(
                "the `ui` feature embeds the built admin GUI, but {} has no index.html. Build it first: \
                 `cd ui && npm ci --ignore-scripts && npm run build`",
                dist.display()
            ));
        }
        collect(&dist, &dist, &mut files)?;
        files.sort();
    }

    let mut out =
        String::from("/// Every file of the built GUI: its path under `dist` and its bytes.\n");
    out.push_str("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (relative, absolute) in &files {
        let absolute = absolute.to_string_lossy();
        writeln!(out, "    ({relative:?}, include_bytes!({absolute:?})),")
            .map_err(|e| e.to_string())?;
    }
    out.push_str("];\n");
    let target =
        PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?).join("ui_assets.rs");
    fs::write(&target, out).map_err(|e| format!("cannot write {}: {e}", target.display()))
}

/// Every regular file under `dir`, as (path relative to `root` with `/` separators, absolute path).
fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            println!("cargo:rerun-if-changed={}", path.display());
            files.push((relative, path));
        }
    }
    Ok(())
}
