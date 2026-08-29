use blake2::{Blake2b512, Digest};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{
    CompilerSharedState, ModuleCapabilities, ModulePermissions, ProjectConfig, Result, TinyOneError,
};

pub(crate) type Resolver = Rc<RefCell<ModuleResolver>>;

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolverInput {
    pub(crate) path: PathBuf,
    pub(crate) digest: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedModule {
    pub(crate) filename: String,
    pub(crate) source: String,
    pub(crate) permissions: ModulePermissions,
}

#[derive(Debug, Clone)]
struct ManifestModule {
    path: String,
    capabilities: ModuleCapabilities,
}

#[derive(Debug)]
pub(crate) struct ModuleResolver {
    sandbox_root: PathBuf,
    config: ProjectConfig,
    base_dirs: HashMap<String, PathBuf>,
    resolutions: HashMap<(String, String), ResolvedModule>,
    sources: HashMap<PathBuf, String>,
    manifests: HashMap<PathBuf, Option<HashMap<String, ManifestModule>>>,
    inputs: BTreeMap<PathBuf, Option<[u8; 16]>>,
    canonical_resolutions: BTreeMap<PathBuf, PathBuf>,
    module_names: BTreeMap<PathBuf, String>,
    module_permissions: HashMap<PathBuf, ModulePermissions>,
    signed_module_valid_until: Option<u64>,
    existing_input_bytes: usize,
}

impl ModuleResolver {
    /// Creates a resolver whose import sandbox is the entry file's directory,
    /// or the nearest ancestor `Config.toml` project root. Canonicalization
    /// rejects `..`, absolute-path, and symlink escapes that already exist
    /// before their source text is read whenever the sandbox is enabled.
    ///
    /// This is import-path containment, not a file-system isolation boundary:
    /// a concurrent actor able to modify the project tree can retarget a path
    /// after canonicalization and before it is read. Put untrusted projects on
    /// a filesystem the attacker cannot modify, or use OS-level isolation,
    /// when adversarial concurrent filesystem writes are in scope.
    pub(crate) fn new(root_file: &Path) -> Result<Self> {
        let root_file = root_file.canonicalize().map_err(|error| {
            TinyOneError::compile(format!(
                "File error: could not canonicalize module root: {error}"
            ))
        })?;
        let config = ProjectConfig::load_for_entry(&root_file)?;
        let sandbox_root = if config.sandbox_enabled() {
            config.project_root().to_path_buf()
        } else {
            root_file
                .parent()
                .ok_or_else(|| {
                    TinyOneError::compile("File error: module root has no parent directory")
                })?
                .to_path_buf()
        };
        let mut resolver = Self {
            sandbox_root,
            config,
            base_dirs: HashMap::new(),
            resolutions: HashMap::new(),
            sources: HashMap::new(),
            manifests: HashMap::new(),
            inputs: BTreeMap::new(),
            canonical_resolutions: BTreeMap::new(),
            module_names: BTreeMap::new(),
            module_permissions: HashMap::new(),
            signed_module_valid_until: None,
            existing_input_bytes: 0,
        };
        if let Some((path, digest)) = resolver.config.input() {
            resolver.inputs.insert(path, Some(digest));
        }
        Ok(resolver)
    }

    pub(crate) fn resolve_import(
        &mut self,
        from_filename: &str,
        import_path: &str,
    ) -> Result<ResolvedModule> {
        let cache_key = (from_filename.to_string(), import_path.to_string());
        if let Some(resolved) = self.resolutions.get(&cache_key) {
            return Ok(resolved.clone());
        }
        let import = Path::new(import_path);
        if self.config.sandbox_enabled()
            && (import.is_absolute()
                || import
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir)))
        {
            return Err(TinyOneError::compile(format!(
                "Import {import_path:?} escapes the module sandbox"
            )));
        }
        reject_native_library_import(import)?;
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
        self.ensure_within_sandbox(&base, "importing module")?;
        if self.config.require_configured_modules() && self.config.module(import_path).is_none() {
            return Err(TinyOneError::compile(format!(
                "Import {import_path:?} is not declared in Config.toml [modules]"
            )));
        }
        let manifest_module = self.resolve_manifest_import(&base, import_path)?;
        let configured_capabilities = manifest_module
            .as_ref()
            .map(|(_, module)| {
                module
                    .capabilities
                    .intersection(self.config.root_capabilities())
            })
            .unwrap_or_else(ModuleCapabilities::none);
        let candidate = manifest_module
            .map(|(directory, module)| directory.join(module.path))
            .unwrap_or_else(|| base.join(import));
        ensure_tinylang_source(&candidate)?;
        reject_native_library_import(&candidate)?;
        let path = if let Some(path) = self.canonical_resolutions.get(&candidate) {
            path.clone()
        } else {
            let path = candidate
                .canonicalize()
                .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
            self.canonical_resolutions
                .insert(candidate.clone(), path.clone());
            path
        };
        self.ensure_within_sandbox(&path, "resolved module")?;
        ensure_tinylang_source(&path)?;
        reject_native_library_import(&path)?;
        let source = if let Some(source) = self.sources.get(&path) {
            source.clone()
        } else {
            let source = read_source_file(&path)
                .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
            self.existing_input_bytes = self.existing_input_bytes.saturating_add(source.len());
            self.inputs
                .insert(path.clone(), Some(content_digest(source.as_bytes())));
            self.sources.insert(path.clone(), source.clone());
            source
        };
        let permissions = if let Some(verification) =
            self.config
                .verify_module_signature(import_path, &path, &source)?
        {
            if !verification
                .declared_capabilities
                .is_subset_of(configured_capabilities)
            {
                return Err(TinyOneError::compile(format!(
                    "Signed module {import_path:?} declares capabilities {:?} that are not all approved by Config.toml [modules.{import_path}].permissions",
                    verification.declared_capabilities.names(),
                )));
            }
            for (signature_input, digest, bytes) in verification.inputs {
                if self.inputs.insert(signature_input, Some(digest)).is_none() {
                    self.existing_input_bytes = self.existing_input_bytes.saturating_add(bytes);
                }
            }
            self.signed_module_valid_until = Some(
                self.signed_module_valid_until
                    .map_or(verification.expires_at, |existing| {
                        existing.min(verification.expires_at)
                    }),
            );
            verification.declared_permissions
        } else {
            ModulePermissions::from_capabilities(configured_capabilities)
        };
        if let Some(existing) = self.module_permissions.get(&path)
            && *existing != permissions
        {
            return Err(TinyOneError::compile(format!(
                "Module {} is imported with conflicting capability grants",
                path.display()
            )));
        }
        self.module_permissions
            .insert(path.clone(), permissions.clone());
        let resolved = ResolvedModule {
            filename: path.to_string_lossy().to_string(),
            source,
            permissions,
        };
        self.resolutions.insert(cache_key, resolved.clone());
        Ok(resolved)
    }

    fn resolve_manifest_import(
        &mut self,
        base: &Path,
        import_path: &str,
    ) -> Result<Option<(PathBuf, ManifestModule)>> {
        if let Some(module) = self.config.module(import_path) {
            return Ok(Some((
                self.config.project_root().to_path_buf(),
                ManifestModule {
                    path: module.path,
                    capabilities: module.capabilities,
                },
            )));
        }
        if !looks_like_module_key(import_path) {
            return Ok(None);
        }
        let sandbox_root = self.sandbox_root.clone();
        for directory in base.ancestors() {
            if self.config.sandbox_enabled() && !directory.starts_with(&sandbox_root) {
                break;
            }
            let manifest_path = directory.join("tinyone.json");
            let modules = self.read_manifest_modules(&manifest_path)?;
            let Some(modules) = modules else {
                continue;
            };
            let Some(target) = modules.get(import_path) else {
                continue;
            };
            return Ok(Some((directory.to_path_buf(), target.clone())));
        }
        Ok(None)
    }

    fn read_manifest_modules(
        &mut self,
        manifest_path: &Path,
    ) -> Result<Option<&HashMap<String, ManifestModule>>> {
        if !self.manifests.contains_key(manifest_path) {
            if !manifest_path.exists() {
                self.inputs.insert(manifest_path.to_path_buf(), None);
                self.manifests.insert(manifest_path.to_path_buf(), None);
            } else {
                let canonical_manifest = manifest_path.canonicalize().map_err(|error| {
                    TinyOneError::compile(format!("Package manifest read error: {error}"))
                })?;
                self.ensure_within_sandbox(&canonical_manifest, "package manifest")?;
                let bytes =
                    read_limited_file(&canonical_manifest, MAX_MANIFEST_BYTES, "Package manifest")?;
                self.existing_input_bytes = self.existing_input_bytes.saturating_add(bytes.len());
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
                    let module = parse_manifest_module(target, name, manifest_path)?;
                    parsed.insert(name.clone(), module);
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

    pub(crate) fn module_count(&self) -> usize {
        self.module_names.len()
    }

    pub(crate) fn existing_input_bytes(&self) -> usize {
        self.existing_input_bytes
    }

    pub(crate) fn has_capability_grants(&self) -> bool {
        self.module_permissions
            .values()
            .any(|permissions| permissions.capabilities() != ModuleCapabilities::none())
    }

    /// The earliest expiry among module signatures used in this compilation.
    /// A cache entry must be rejected once this moment has passed, even when
    /// the source and sidecar digests are otherwise unchanged.
    pub(crate) const fn signed_module_valid_until(&self) -> Option<u64> {
        self.signed_module_valid_until
    }

    pub(crate) const fn root_capabilities(&self) -> ModuleCapabilities {
        self.config.root_capabilities()
    }

    pub(crate) const fn vm_settings(&self) -> crate::VmSettings {
        self.config.vm_settings()
    }

    fn ensure_within_sandbox(&self, path: &Path, kind: &str) -> Result<()> {
        if !self.config.sandbox_enabled() {
            return Ok(());
        }
        if path.starts_with(&self.sandbox_root) {
            Ok(())
        } else {
            Err(TinyOneError::compile(format!(
                "{kind} {} escapes module sandbox root {}",
                path.display(),
                self.sandbox_root.display()
            )))
        }
    }
}

fn parse_manifest_module(
    value: &JsonValue,
    name: &str,
    manifest_path: &Path,
) -> Result<ManifestModule> {
    let (path, capability_values) = if let Some(path) = value.as_str() {
        (path.to_string(), Vec::new())
    } else {
        let object = value.as_object().ok_or_else(|| {
            TinyOneError::compile(format!(
                "Package manifest module {name:?} in {} must be a source path string or object",
                manifest_path.display()
            ))
        })?;
        let path = object
            .get("path")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Package manifest module {name:?} in {} must contain a string path",
                    manifest_path.display()
                ))
            })?;
        let capability_values = object
            .get("capabilities")
            .map(|value| {
                value.as_array().ok_or_else(|| {
                    TinyOneError::compile(format!(
                        "Package manifest module {name:?} capabilities in {} must be a list",
                        manifest_path.display()
                    ))
                })
            })
            .transpose()?
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        TinyOneError::compile(format!(
                            "Package manifest module {name:?} capabilities in {} must contain strings",
                            manifest_path.display()
                        ))
                    }))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        (path.to_string(), capability_values)
    };
    let target = Path::new(&path);
    if target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(TinyOneError::compile(format!(
            "Package manifest module {name:?} in {} has an escaping path {path:?}",
            manifest_path.display()
        )));
    }
    reject_native_library_import(target)?;
    ensure_tinylang_source(target)?;
    let capabilities = ModuleCapabilities::from_names(&capability_values)?;
    Ok(ManifestModule { path, capabilities })
}

fn ensure_tinylang_source(path: &Path) -> Result<()> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("to"))
    {
        Ok(())
    } else {
        Err(TinyOneError::compile(format!(
            "Module import {:?} rejected: modules must be TinyLang .to source files",
            path.display()
        )))
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

        let mut resolver = ModuleResolver::new(&main).expect("create resolver");
        let first = resolver
            .resolve_import(&main.to_string_lossy(), "first")
            .expect("resolve first");
        let second = resolver
            .resolve_import(&main.to_string_lossy(), "second")
            .expect("resolve second");
        let again = resolver
            .resolve_import(&main.to_string_lossy(), "first")
            .expect("resolve cached first");

        assert_eq!(first.filename, second.filename);
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
