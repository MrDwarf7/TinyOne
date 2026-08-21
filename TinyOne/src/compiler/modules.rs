use blake2::{Blake2b512, Digest};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{CompilerSharedState, Result, TinyOneError};

pub(crate) type Resolver = Rc<RefCell<ModuleResolver>>;

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolverInput {
    pub(crate) path: PathBuf,
    pub(crate) digest: Option<[u8; 16]>,
}

#[derive(Debug, Default)]
pub(crate) struct ModuleResolver {
    base_dirs: HashMap<String, PathBuf>,
    resolutions: HashMap<(String, String), (String, String)>,
    sources: HashMap<PathBuf, String>,
    manifests: HashMap<PathBuf, Option<HashMap<String, String>>>,
    inputs: BTreeMap<PathBuf, Option<[u8; 16]>>,
    canonical_resolutions: BTreeMap<PathBuf, PathBuf>,
    module_names: BTreeMap<PathBuf, String>,
}

impl ModuleResolver {
    pub(crate) fn resolve_import(
        &mut self,
        from_filename: &str,
        import_path: &str,
    ) -> Result<(String, String)> {
        let cache_key = (from_filename.to_string(), import_path.to_string());
        if let Some(resolved) = self.resolutions.get(&cache_key) {
            return Ok(resolved.clone());
        }
        reject_native_library_import(Path::new(import_path))?;
        let base = if let Some(base) = self.base_dirs.get(from_filename) {
            base.clone()
        } else {
            let base = Path::new(from_filename)
                .canonicalize()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            self.base_dirs
                .insert(from_filename.to_string(), base.clone());
            base
        };
        let candidate = self
            .resolve_manifest_import(&base, import_path)?
            .unwrap_or_else(|| base.join(import_path));
        reject_native_library_import(&candidate)?;
        let path = candidate
            .canonicalize()
            .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
        reject_native_library_import(&path)?;
        self.canonical_resolutions.insert(candidate, path.clone());
        let source = if let Some(source) = self.sources.get(&path) {
            source.clone()
        } else {
            let source = read_source_file(&path)
                .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
            self.inputs
                .insert(path.clone(), Some(content_digest(source.as_bytes())));
            self.sources.insert(path.clone(), source.clone());
            source
        };
        let resolved = (path.to_string_lossy().to_string(), source);
        self.resolutions.insert(cache_key, resolved.clone());
        Ok(resolved)
    }

    fn resolve_manifest_import(
        &mut self,
        base: &Path,
        import_path: &str,
    ) -> Result<Option<PathBuf>> {
        if !looks_like_module_key(import_path) {
            return Ok(None);
        }
        for directory in base.ancestors() {
            let manifest_path = directory.join("tinyone.json");
            let modules = self.read_manifest_modules(&manifest_path)?;
            let Some(modules) = modules else {
                continue;
            };
            let Some(target) = modules.get(import_path) else {
                continue;
            };
            return Ok(Some(directory.join(target)));
        }
        Ok(None)
    }

    fn read_manifest_modules(
        &mut self,
        manifest_path: &Path,
    ) -> Result<Option<&HashMap<String, String>>> {
        if !self.manifests.contains_key(manifest_path) {
            if !manifest_path.exists() {
                self.inputs.insert(manifest_path.to_path_buf(), None);
                self.manifests.insert(manifest_path.to_path_buf(), None);
            } else {
                let bytes =
                    read_limited_file(manifest_path, MAX_MANIFEST_BYTES, "Package manifest")?;
                self.inputs
                    .insert(manifest_path.to_path_buf(), Some(content_digest(&bytes)));
                let text = String::from_utf8(bytes).map_err(|error| {
                    TinyOneError::compile(format!("Package manifest must be UTF-8: {error}"))
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
                let mut parsed = HashMap::with_capacity(modules.len());
                for (name, target) in modules {
                    let target = target.as_str().ok_or_else(|| {
                        TinyOneError::compile(format!(
                            "Package manifest module {name:?} in {} must be a string",
                            manifest_path.display()
                        ))
                    })?;
                    parsed.insert(name.clone(), target.to_string());
                }
                self.manifests
                    .insert(manifest_path.to_path_buf(), Some(parsed));
            }
        }
        Ok(self.manifests.get(manifest_path).and_then(Option::as_ref))
    }

    pub(crate) fn inputs(&self) -> Vec<ResolverInput> {
        self.inputs
            .iter()
            .map(|(path, digest)| ResolverInput {
                path: path.clone(),
                digest: *digest,
            })
            .collect()
    }

    pub(crate) fn canonical_resolutions(&self) -> Vec<(PathBuf, PathBuf)> {
        self.canonical_resolutions
            .iter()
            .map(|(candidate, canonical)| (candidate.clone(), canonical.clone()))
            .collect()
    }

    pub(crate) fn record_module_name(&mut self, filename: &str, module_name: &str) {
        self.module_names
            .insert(PathBuf::from(filename), module_name.to_string());
    }

    pub(crate) fn module_names(&self) -> Vec<(PathBuf, String)> {
        self.module_names
            .iter()
            .map(|(path, name)| (path.clone(), name.clone()))
            .collect()
    }
}

pub(crate) fn read_source_file(path: &Path) -> Result<String> {
    let bytes = read_limited_file(path, MAX_SOURCE_BYTES, "Source")?;
    String::from_utf8(bytes)
        .map_err(|error| TinyOneError::compile(format!("Source must be UTF-8: {error}")))
}

fn reject_native_library_import(path: &Path) -> Result<()> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_native = filename.ends_with(".dll")
        || filename.ends_with(".dylib")
        || filename.ends_with(".so")
        || filename.contains(".so.");
    if is_native {
        return Err(TinyOneError::compile(format!(
            "Native module import {:?} rejected: direct DLL/SO loading bypasses TinyOne bytecode verification; use a versioned native-module ABI or an isolated worker boundary",
            path.display()
        )));
    }
    Ok(())
}

fn read_limited_file(path: &Path, max_bytes: usize, kind: &str) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)
        .map_err(|error| TinyOneError::compile(format!("{kind} read error: {error}")))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| TinyOneError::compile(format!("{kind} read error: {error}")))?;
    if bytes.len() > max_bytes {
        return Err(TinyOneError::compile(format!(
            "{kind} rejected: byte size limit {max_bytes} exceeded"
        )));
    }
    Ok(bytes)
}

pub(crate) fn content_digest(bytes: &[u8]) -> [u8; 16] {
    let digest = Blake2b512::digest(bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "tinyone-resolver-cache-{}-{stamp}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn session_cache_reuses_manifest_resolution_and_source_text() {
        let temp = TempDir::new();
        let main = temp.0.join("main.to");
        let module = temp.0.join("shared.to");
        fs::write(&main, "").expect("write main");
        fs::write(&module, "export fn value() { return 1 }").expect("write module");
        fs::write(
            temp.0.join("tinyone.json"),
            r#"{"modules":{"first":"shared.to","second":"shared.to"}}"#,
        )
        .expect("write manifest");

        let mut resolver = ModuleResolver::default();
        let first = resolver
            .resolve_import(&main.to_string_lossy(), "first")
            .expect("resolve first");
        let second = resolver
            .resolve_import(&main.to_string_lossy(), "second")
            .expect("resolve second");
        let again = resolver
            .resolve_import(&main.to_string_lossy(), "first")
            .expect("resolve cached first");

        assert_eq!(first.0, second.0);
        assert_eq!(first, again);
        assert_eq!(1, resolver.sources.len());
        assert_eq!(
            1,
            resolver
                .manifests
                .values()
                .filter(|item| item.is_some())
                .count()
        );
        assert_eq!(2, resolver.resolutions.len());
    }
}
