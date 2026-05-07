use blake2::{Blake2b512, Digest};
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{CompilerSharedState, Result, TinyOneError};

pub(crate) type Resolver = fn(&str, &str) -> Result<(String, String)>;

pub(crate) fn resolve_import(from_filename: &str, import_path: &str) -> Result<(String, String)> {
    let base = Path::new(from_filename)
        .canonicalize()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let path = resolve_manifest_import(&base, import_path)?
        .unwrap_or_else(|| base.join(import_path))
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
    Ok((path.to_string_lossy().to_string(), source))
}

fn resolve_manifest_import(base: &Path, import_path: &str) -> Result<Option<PathBuf>> {
    if !looks_like_module_key(import_path) {
        return Ok(None);
    }
    for directory in base.ancestors() {
        let manifest_path = directory.join("tinyone.json");
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path).map_err(|error| {
            TinyOneError::compile(format!("Package manifest read error: {error}"))
        })?;
        let data: JsonValue = serde_json::from_str(&text).map_err(|error| {
            TinyOneError::compile(format!("Package manifest JSON error: {error}"))
        })?;
        let modules = data
            .get("modules")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Package manifest {} must contain a modules object",
                    manifest_path.display()
                ))
            })?;
        let Some(target) = modules.get(import_path) else {
            continue;
        };
        let target = target.as_str().ok_or_else(|| {
            TinyOneError::compile(format!(
                "Package manifest module {import_path:?} in {} must be a string",
                manifest_path.display()
            ))
        })?;
        return Ok(Some(directory.join(target)));
    }
    Ok(None)
}

fn module_name_from_filename(filename: &str) -> String {
    sanitize_identifier(
        Path::new(filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("module"),
    )
}

pub(crate) fn module_name_from_import(import_path: &str, filename: &str) -> String {
    if looks_like_module_key(import_path) {
        sanitize_identifier(import_path)
    } else {
        module_name_from_filename(filename)
    }
}

pub(crate) fn unique_module_name(
    state: &mut CompilerSharedState,
    base_name: &str,
    filename: &str,
) -> String {
    if state
        .module_name_owners
        .get(base_name)
        .map(|owner| owner == filename)
        .unwrap_or(true)
    {
        state
            .module_name_owners
            .insert(base_name.to_string(), filename.to_string());
        return base_name.to_string();
    }
    let digest = Blake2b512::digest(filename.as_bytes());
    let suffix = hex::encode(&digest[..4]);
    let mut name = format!("{base_name}_{suffix}");
    while state
        .module_name_owners
        .get(&name)
        .map(|owner| owner != filename)
        .unwrap_or(false)
    {
        let digest = Blake2b512::digest(format!("{filename}:{suffix}").as_bytes());
        name = format!("{}_{}", base_name, hex::encode(&digest[..4]));
    }
    state
        .module_name_owners
        .insert(name.clone(), filename.to_string());
    name
}

pub(crate) fn default_import_alias(import_path: &str) -> String {
    if looks_like_module_key(import_path) {
        sanitize_identifier(import_path)
    } else {
        sanitize_identifier(
            Path::new(import_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("module"),
        )
    }
}

fn looks_like_module_key(import_path: &str) -> bool {
    !import_path.contains('/')
        && !import_path.contains('\\')
        && !import_path.starts_with('.')
        && !import_path.contains('.')
}

fn sanitize_identifier(text: &str) -> String {
    let mut out = text
        .chars()
        .map(|ch| {
            if ch == '_' || ch.is_alphanumeric() {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if out.is_empty() || out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out = format!("module_{out}");
    }
    out
}
