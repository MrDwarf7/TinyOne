use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest as ShaDigest, Sha256};
use toml::Value as TomlValue;

use crate::{ModuleCapabilities, ModulePermissions, Result, TinyOneError, VmSettings, content_digest};

const CONFIG_FILE: &str = "Config.toml";
const MODULE_MANIFEST_FILE: &str = "module.toml";
const MODULE_SIGNATURE_FILE: &str = "signature.toml";
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_MODULE_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
const MODULE_SIGNATURE_POLICY_VERSION: &str = "1";
const TINYONE_LANGUAGE_VERSION: &str = "1";
const MAX_MODULE_SIGNATURE_VALIDITY_SECS: u64 = 366 * 24 * 60 * 60;
const CENTRAL_ROOTS_ENV: Option<&str> = option_env!("TINYONE_CENTRAL_AUTHORITY_ROOTS");

/// A module entry declared by the project configuration. `Config.toml` is
/// deliberately authoritative when it names a module; legacy `tinyone.json`
/// entries remain available for projects that have not migrated yet.
#[derive(Debug, Clone)]
pub(crate) struct ConfigModule {
    pub(crate) path:         String,
    pub(crate) capabilities: ModuleCapabilities,
}

#[derive(Debug, Clone, Default)]
struct SigningPolicy {
    require_module_signatures: bool,
    authorities:               HashMap<String, AuthorityCertificate>,
}

/// A company module-signing key that a TinyOne central root has certified.
/// It is verified once while reading Config.toml, then reused for every
/// module signed by that company.
#[derive(Debug, Clone)]
struct AuthorityCertificate {
    public_key: VerifyingKey,
}

/// The declaration carried by `module.toml`. It is intentionally separate
/// from the artifact and signature data: a package has one declaration, while
/// each published artifact can be signed and rotated independently.
#[derive(Debug, Clone)]
struct ModuleManifest {
    name:                    String,
    version:                 String,
    publisher:               String,
    purpose:                 String,
    network_outbound:        bool,
    network_listen:          bool,
    filesystem_read:         bool,
    filesystem_write:        bool,
    process_spawn:           bool,
    environment_read:        Vec<String>,
    ffi_allowed:             bool,
    graphics_gpu:            bool,
    hardware_access:         bool,
    threads_allowed:         bool,
    unsafe_memory_allowed:   bool,
    linux_pipelines_allowed: bool,
    dependencies:            BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct SignatureMetadata {
    source_hash:          String,
    binary_hash:          Option<String>,
    dependency_lock_hash: String,
    compiler_version:     String,
    language_version:     String,
    authority_id:         String,
    policy_version:       String,
    issued_at:            u64,
    expires_at:           u64,
    signing_record_id:    String,
    signature:            Option<Signature>,
}

/// Data the resolver must retain as compiler-cache inputs after a signed
/// module has been checked.
#[derive(Debug, Clone)]
pub(crate) struct SignatureVerification {
    pub(crate) inputs:                Vec<(PathBuf, [u8; 16], usize)>,
    pub(crate) declared_capabilities: ModuleCapabilities,
    pub(crate) declared_permissions:  ModulePermissions,
    pub(crate) expires_at:            u64,
}

/// Policy discovered from the nearest ancestor `Config.toml`.
#[derive(Debug, Clone)]
pub(crate) struct ProjectConfig {
    project_root:               PathBuf,
    sandbox_enabled:            bool,
    require_configured_modules: bool,
    root_capabilities:          ModuleCapabilities,
    vm_settings:                VmSettings,
    modules:                    HashMap<String, ConfigModule>,
    signing:                    SigningPolicy,
    input:                      Option<(PathBuf, [u8; 16])>,
}

impl ProjectConfig {
    /// Loads the closest `Config.toml` above the entry point, matching the
    /// project-root discovery users expect from Cargo. Without a config the
    /// legacy safe behavior remains: imports are confined to the entry file's
    /// directory and root code retains the host embedding's authority.
    pub(crate) fn load_for_entry(entry_file: &Path) -> Result<Self> {
        let entry_parent = entry_file
            .parent()
            .ok_or_else(|| TinyOneError::compile("File error: module root has no parent directory"))?;
        for directory in entry_parent.ancestors() {
            let config_path = directory.join(CONFIG_FILE);
            if !config_path.is_file() {
                continue;
            }
            let root = directory
                .canonicalize()
                .map_err(|error| TinyOneError::compile(format!("Config.toml project root error: {error}")))?;
            let canonical_config = config_path
                .canonicalize()
                .map_err(|error| TinyOneError::compile(format!("Config.toml read error: {error}")))?;
            if canonical_config.parent() != Some(root.as_path()) {
                return Err(TinyOneError::compile("Config.toml must not be a symlink outside its project root"));
            }
            let bytes = read_limited(&canonical_config, MAX_CONFIG_BYTES, "Config.toml")?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| TinyOneError::compile(format!("Config.toml must be UTF-8: {error}")))?;
            let value: TomlValue = text
                .parse()
                .map_err(|error| TinyOneError::compile(format!("Config.toml parse error: {error}")))?;
            return Self::parse(root, canonical_config, bytes, value);
        }

        Ok(Self {
            project_root:               entry_parent.to_path_buf(),
            sandbox_enabled:            true,
            require_configured_modules: false,
            root_capabilities:          ModuleCapabilities::all(),
            vm_settings:                VmSettings::default(),
            modules:                    HashMap::new(),
            signing:                    SigningPolicy::default(),
            input:                      None,
        })
    }

    fn parse(project_root: PathBuf, config_path: PathBuf, bytes: Vec<u8>, value: TomlValue) -> Result<Self> {
        let root = value
            .as_table()
            .ok_or_else(|| TinyOneError::compile("Config.toml must contain a top-level table"))?;
        reject_unknown_fields(
            root,
            &["package", "vm", "sandbox", "permissions", "rules", "modules", "signing"],
            "Config.toml",
        )?;
        validate_package(root.get("package"))?;

        let sandbox = optional_table(root.get("sandbox"), "sandbox")?;
        if let Some(sandbox) = sandbox {
            reject_unknown_fields(sandbox, &["enabled"], "sandbox")?;
        }
        let sandbox_enabled = sandbox
            .map(|table| optional_bool(table.get("enabled"), "sandbox.enabled"))
            .transpose()?
            .flatten()
            .unwrap_or(true);

        let rules = optional_table(root.get("rules"), "rules")?;
        if let Some(rules) = rules {
            reject_unknown_fields(rules, &["require_configured_modules"], "rules")?;
        }
        let require_configured_modules = rules
            .map(|table| optional_bool(table.get("require_configured_modules"), "rules.require_configured_modules"))
            .transpose()?
            .flatten()
            .unwrap_or(false);

        let permissions = optional_table(root.get("permissions"), "permissions")?;
        if let Some(permissions) = permissions {
            reject_unknown_fields(permissions, &["codebase"], "permissions")?;
        }
        let root_capabilities = permissions
            .and_then(|table| table.get("codebase"))
            .map(|value| parse_capability_list(value, "permissions.codebase"))
            .transpose()?
            .unwrap_or_else(ModuleCapabilities::all);

        let vm = optional_table(root.get("vm"), "vm")?;
        if let Some(vm) = vm {
            reject_unknown_fields(vm, &["max_call_depth"], "vm")?;
        }
        let vm_settings = vm
            .and_then(|table| table.get("max_call_depth"))
            .map(|value| VmSettings::with_max_call_depth(parse_usize(value, "vm.max_call_depth")?))
            .transpose()?
            .unwrap_or_default();

        let modules = parse_modules(root.get("modules"))?;
        let signing = parse_signing(root.get("signing"))?;

        Ok(Self {
            project_root,
            sandbox_enabled,
            require_configured_modules,
            root_capabilities,
            vm_settings,
            modules,
            signing,
            input: Some((config_path, content_digest(&bytes))),
        })
    }

    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) const fn sandbox_enabled(&self) -> bool {
        self.sandbox_enabled
    }

    pub(crate) const fn require_configured_modules(&self) -> bool {
        self.require_configured_modules
    }

    pub(crate) const fn root_capabilities(&self) -> ModuleCapabilities {
        self.root_capabilities
    }

    pub(crate) const fn vm_settings(&self) -> VmSettings {
        self.vm_settings
    }

    pub(crate) fn module(&self, name: &str) -> Option<ConfigModule> {
        self.modules.get(name).cloned()
    }

    pub(crate) fn input(&self) -> Option<(PathBuf, [u8; 16])> {
        self.input.clone()
    }

    pub(crate) fn verify_module_signature(
        &self,
        module_name: &str,
        module_path: &Path,
        source: &str,
    ) -> Result<Option<SignatureVerification>> {
        if !self.signing.require_module_signatures {
            return Ok(None);
        }
        let module_directory = module_path.parent().ok_or_else(|| {
            TinyOneError::compile(format!("Signed module {} has no containing directory", module_path.display()))
        })?;
        let manifest_path = canonical_sidecar(module_directory, MODULE_MANIFEST_FILE, "module.toml")?;
        let signature_path = canonical_sidecar(module_directory, MODULE_SIGNATURE_FILE, "signature.toml")?;
        let manifest_bytes = read_limited(&manifest_path, MAX_MODULE_MANIFEST_BYTES, "Module manifest")?;
        let manifest = parse_module_manifest(&manifest_bytes)?;
        if import_name_requires_manifest_match(module_name) && manifest.name != module_name {
            return Err(TinyOneError::compile(format!(
                "Signed module imported as {module_name:?} declares module.name {:?}",
                manifest.name
            )));
        }
        let bytes = read_limited(&signature_path, MAX_SIGNATURE_BYTES, "Module signature")?;
        let metadata = parse_signature_metadata(&bytes, true)?;
        if metadata.source_hash != sha256_prefixed(source.as_bytes()) {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} source_hash does not match its exact UTF-8 source"
            )));
        }
        let expected_lock_hash = dependency_lock_hash(&manifest.dependencies);
        if metadata.dependency_lock_hash != expected_lock_hash {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} dependency_lock_hash does not match module.toml [dependencies]"
            )));
        }
        if metadata.compiler_version != env!("CARGO_PKG_VERSION") {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} targets compiler version {:?}, but this TinyOne build is {:?}",
                metadata.compiler_version,
                env!("CARGO_PKG_VERSION")
            )));
        }
        if metadata.language_version != TINYONE_LANGUAGE_VERSION {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} targets language version {:?}, but this runtime supports {:?}",
                metadata.language_version, TINYONE_LANGUAGE_VERSION
            )));
        }
        if metadata.policy_version != MODULE_SIGNATURE_POLICY_VERSION {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} uses policy version {:?}, but this runtime supports {:?}",
                metadata.policy_version, MODULE_SIGNATURE_POLICY_VERSION
            )));
        }
        let now = unix_time_now()?;
        if metadata.expires_at <= metadata.issued_at
            || metadata.expires_at - metadata.issued_at > MAX_MODULE_SIGNATURE_VALIDITY_SECS
            || now < metadata.issued_at
            || now > metadata.expires_at
        {
            return Err(TinyOneError::compile(format!(
                "Signed module {module_name:?} has an invalid, expired, or not-yet-valid signing interval"
            )));
        }
        let authority = self.signing.authorities.get(&metadata.authority_id).ok_or_else(|| {
            TinyOneError::compile(format!(
                "Module signing authority {:?} is not certified by a TinyOne central root",
                metadata.authority_id
            ))
        })?;
        let digest = module_signature_digest_from_parts(&manifest, &metadata);
        let signature = metadata.signature.as_ref().ok_or_else(|| {
            TinyOneError::compile("signature.toml signing.signature is required for module verification")
        })?;
        authority.public_key.verify(&digest, signature).map_err(|_| {
            TinyOneError::compile(format!("Module {module_name:?} failed Ed25519 signature verification"))
        })?;
        let declared_permissions = manifest.runtime_permissions();
        Ok(Some(SignatureVerification {
            inputs: vec![
                (manifest_path, content_digest(&manifest_bytes), manifest_bytes.len()),
                (signature_path, content_digest(&bytes), bytes.len()),
            ],
            declared_capabilities: declared_permissions.capabilities(),
            declared_permissions,
            expires_at: metadata.expires_at,
        }))
    }
}

impl ModuleManifest {
    fn runtime_permissions(&self) -> ModulePermissions {
        ModulePermissions::from_signed_manifest(
            self.filesystem_read,
            self.filesystem_write,
            self.environment_read.clone(),
            self.network_outbound,
            self.network_listen,
            self.process_spawn,
            self.ffi_allowed,
            self.graphics_gpu,
            self.hardware_access,
            self.threads_allowed,
            self.unsafe_memory_allowed,
            self.linux_pipelines_allowed,
        )
    }
}

/// Produces the exact deterministic byte representation that authorities sign
/// after hashing it with SHA-256. The signature field itself is intentionally
/// excluded, so an authority can create an unsigned `signature.toml`, obtain
/// this payload/digest, and then write the resulting signature into the file.
pub fn canonical_module_signature_payload(module_manifest_toml: &str, signature_toml: &str) -> Result<Vec<u8>> {
    let manifest = parse_module_manifest(module_manifest_toml.as_bytes())?;
    let metadata = parse_signature_metadata(signature_toml.as_bytes(), false)?;
    Ok(canonical_module_signature_payload_from_parts(&manifest, &metadata))
}

/// SHA-256 digest to be signed by an Ed25519 authority for a module package.
/// This does not access private keys and therefore is safe to use in package
/// tooling or an offline signing request.
pub fn module_signature_digest(module_manifest_toml: &str, signature_toml: &str) -> Result<[u8; 32]> {
    let payload = canonical_module_signature_payload(module_manifest_toml, signature_toml)?;
    Ok(sha256_digest(&payload))
}

/// Canonical `sha256:` hash for the UTF-8 source artifact named in
/// `signature.toml [artifact].source_hash`.
pub fn module_source_hash(source: &[u8]) -> String {
    sha256_prefixed(source)
}

/// Canonical dependency-lock hash for the dependencies declared in a
/// `module.toml`. Package tooling writes this value into
/// `signature.toml [artifact].dependency_lock_hash` before obtaining the
/// Ed25519 signature.
pub fn module_dependency_lock_hash(module_manifest_toml: &str) -> Result<String> {
    let manifest = parse_module_manifest(module_manifest_toml.as_bytes())?;
    Ok(dependency_lock_hash(&manifest.dependencies))
}

fn module_signature_digest_from_parts(manifest: &ModuleManifest, metadata: &SignatureMetadata) -> [u8; 32] {
    sha256_digest(&canonical_module_signature_payload_from_parts(manifest, metadata))
}

fn canonical_module_signature_payload_from_parts(manifest: &ModuleManifest, metadata: &SignatureMetadata) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1024 + manifest.purpose.len());
    payload.extend_from_slice(b"tinyone-module-signature-v1\0");
    canonical_string(&mut payload, "module_name", &manifest.name);
    canonical_string(&mut payload, "module_version", &manifest.version);
    canonical_string(&mut payload, "publisher_id", &manifest.publisher);
    canonical_string(&mut payload, "declared_purpose", &manifest.purpose);
    canonical_bool(&mut payload, "capabilities.network.outbound", manifest.network_outbound);
    canonical_bool(&mut payload, "capabilities.network.listen", manifest.network_listen);
    canonical_bool(&mut payload, "capabilities.filesystem.read", manifest.filesystem_read);
    canonical_bool(&mut payload, "capabilities.filesystem.write", manifest.filesystem_write);
    canonical_bool(&mut payload, "capabilities.process.spawn", manifest.process_spawn);
    canonical_strings(&mut payload, "capabilities.environment.read", &manifest.environment_read);
    canonical_bool(&mut payload, "capabilities.ffi.allowed", manifest.ffi_allowed);
    canonical_bool(&mut payload, "capabilities.graphics.gpu", manifest.graphics_gpu);
    canonical_bool(&mut payload, "capabilities.hardware.access", manifest.hardware_access);
    canonical_bool(&mut payload, "capabilities.threads.allowed", manifest.threads_allowed);
    canonical_bool(&mut payload, "capabilities.memory.unsafe", manifest.unsafe_memory_allowed);
    canonical_bool(&mut payload, "capabilities.pipelines.linux", manifest.linux_pipelines_allowed);
    canonical_dependencies(&mut payload, &manifest.dependencies);
    canonical_string(&mut payload, "source_hash", &metadata.source_hash);
    canonical_string(&mut payload, "artifact_hash", metadata.binary_hash.as_deref().unwrap_or(""));
    canonical_string(&mut payload, "dependency_lock_hash", &metadata.dependency_lock_hash);
    canonical_string(&mut payload, "compiler_version", &metadata.compiler_version);
    canonical_string(&mut payload, "language_version", &metadata.language_version);
    canonical_string(&mut payload, "authority_id", &metadata.authority_id);
    canonical_string(&mut payload, "policy_version", &metadata.policy_version);
    canonical_u64(&mut payload, "issued_at", metadata.issued_at);
    canonical_u64(&mut payload, "expires_at", metadata.expires_at);
    canonical_string(&mut payload, "signing_record_id", &metadata.signing_record_id);
    payload
}

fn canonical_field(payload: &mut Vec<u8>, name: &str, value: &[u8]) {
    payload.extend_from_slice(&(name.len() as u16).to_be_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(&(value.len() as u32).to_be_bytes());
    payload.extend_from_slice(value);
}

fn canonical_string(payload: &mut Vec<u8>, name: &str, value: &str) {
    canonical_field(payload, name, value.as_bytes());
}

fn canonical_bool(payload: &mut Vec<u8>, name: &str, value: bool) {
    canonical_field(payload, name, &[u8::from(value)]);
}

fn canonical_u64(payload: &mut Vec<u8>, name: &str, value: u64) {
    canonical_field(payload, name, &value.to_be_bytes());
}

fn canonical_strings(payload: &mut Vec<u8>, name: &str, values: &[String]) {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&(values.len() as u32).to_be_bytes());
    for value in values {
        encoded.extend_from_slice(&(value.len() as u32).to_be_bytes());
        encoded.extend_from_slice(value.as_bytes());
    }
    canonical_field(payload, name, &encoded);
}

fn canonical_dependencies(payload: &mut Vec<u8>, dependencies: &BTreeMap<String, String>) {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&(dependencies.len() as u32).to_be_bytes());
    for (name, version) in dependencies {
        encoded.extend_from_slice(&(name.len() as u32).to_be_bytes());
        encoded.extend_from_slice(name.as_bytes());
        encoded.extend_from_slice(&(version.len() as u32).to_be_bytes());
        encoded.extend_from_slice(version.as_bytes());
    }
    canonical_field(payload, "dependencies", &encoded);
}

fn parse_module_manifest(bytes: &[u8]) -> Result<ModuleManifest> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| TinyOneError::compile(format!("module.toml must be UTF-8: {error}")))?;
    let value: TomlValue = text
        .parse()
        .map_err(|error| TinyOneError::compile(format!("module.toml parse error: {error}")))?;
    let root = value
        .as_table()
        .ok_or_else(|| TinyOneError::compile("module.toml must contain a top-level table"))?;
    reject_unknown_fields(root, &["module", "purpose", "capabilities", "dependencies"], "module.toml")?;
    let module = required_toml_table(root.get("module"), "module.toml [module]")?;
    reject_unknown_fields(module, &["name", "version", "publisher"], "module.toml module")?;
    let name = required_metadata_text(module.get("name"), "module.toml module.name", 256)?;
    let version = required_metadata_text(module.get("version"), "module.toml module.version", 256)?;
    let publisher = required_metadata_text(module.get("publisher"), "module.toml module.publisher", 256)?;
    validate_authority_id(&publisher, "module.toml module.publisher")?;

    let purpose = required_toml_table(root.get("purpose"), "module.toml [purpose]")?;
    reject_unknown_fields(purpose, &["description"], "module.toml purpose")?;
    let purpose = required_metadata_text(purpose.get("description"), "module.toml purpose.description", 8192)?;

    let capabilities = optional_toml_table(root.get("capabilities"), "module.toml capabilities")?;
    let network = nested_capability_table(capabilities, "network", &["outbound", "listen"])?;
    let filesystem = nested_capability_table(capabilities, "filesystem", &["read", "write"])?;
    let process = nested_capability_table(capabilities, "process", &["spawn"])?;
    let environment = nested_capability_table(capabilities, "environment", &["read"])?;
    let ffi = nested_capability_table(capabilities, "ffi", &["allowed"])?;
    let graphics = nested_capability_table(capabilities, "graphics", &["gpu"])?;
    let hardware = nested_capability_table(capabilities, "hardware", &["access"])?;
    let threads = nested_capability_table(capabilities, "threads", &["allowed"])?;
    let memory = nested_capability_table(capabilities, "memory", &["unsafe"])?;
    let pipelines = nested_capability_table(capabilities, "pipelines", &["linux"])?;
    if let Some(capabilities) = capabilities {
        reject_unknown_fields(
            capabilities,
            &[
                "network",
                "filesystem",
                "process",
                "environment",
                "ffi",
                "graphics",
                "hardware",
                "threads",
                "memory",
                "pipelines",
            ],
            "module.toml capabilities",
        )?;
    }

    let mut environment_read = environment
        .and_then(|table| table.get("read"))
        .map(|value| parse_environment_names(value, "module.toml capabilities.environment.read"))
        .transpose()?
        .unwrap_or_default();
    environment_read.sort();
    let dependencies = parse_dependencies(root.get("dependencies"))?;

    Ok(ModuleManifest {
        name,
        version,
        publisher,
        purpose,
        network_outbound: capability_bool(network, "outbound", "module.toml capabilities.network.outbound")?,
        network_listen: capability_bool(network, "listen", "module.toml capabilities.network.listen")?,
        filesystem_read: capability_bool(filesystem, "read", "module.toml capabilities.filesystem.read")?,
        filesystem_write: capability_bool(filesystem, "write", "module.toml capabilities.filesystem.write")?,
        process_spawn: capability_bool(process, "spawn", "module.toml capabilities.process.spawn")?,
        environment_read,
        ffi_allowed: capability_bool(ffi, "allowed", "module.toml capabilities.ffi.allowed")?,
        graphics_gpu: capability_bool(graphics, "gpu", "module.toml capabilities.graphics.gpu")?,
        hardware_access: capability_bool(hardware, "access", "module.toml capabilities.hardware.access")?,
        threads_allowed: capability_bool(threads, "allowed", "module.toml capabilities.threads.allowed")?,
        unsafe_memory_allowed: capability_bool(memory, "unsafe", "module.toml capabilities.memory.unsafe")?,
        linux_pipelines_allowed: capability_bool(pipelines, "linux", "module.toml capabilities.pipelines.linux")?,
        dependencies,
    })
}

fn parse_signature_metadata(bytes: &[u8], signature_required: bool) -> Result<SignatureMetadata> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| TinyOneError::compile(format!("signature.toml must be UTF-8: {error}")))?;
    let value: TomlValue = text
        .parse()
        .map_err(|error| TinyOneError::compile(format!("signature.toml parse error: {error}")))?;
    let root = value
        .as_table()
        .ok_or_else(|| TinyOneError::compile("signature.toml must contain a top-level table"))?;
    reject_unknown_fields(root, &["artifact", "signing"], "signature.toml")?;
    let artifact = required_toml_table(root.get("artifact"), "signature.toml [artifact]")?;
    reject_unknown_fields(
        artifact,
        &[
            "source_hash",
            "binary_hash",
            "dependency_lock_hash",
            "compiler_version",
            "language_version",
        ],
        "signature.toml artifact",
    )?;
    let source_hash = required_metadata_text(artifact.get("source_hash"), "signature.toml artifact.source_hash", 72)?;
    validate_sha256_hash(&source_hash, "signature.toml artifact.source_hash")?;
    let binary_hash = artifact
        .get("binary_hash")
        .map(|value| required_metadata_text(Some(value), "signature.toml artifact.binary_hash", 72))
        .transpose()?;
    if let Some(binary_hash) = &binary_hash {
        validate_sha256_hash(binary_hash, "signature.toml artifact.binary_hash")?;
    }
    let dependency_lock_hash = required_metadata_text(
        artifact.get("dependency_lock_hash"),
        "signature.toml artifact.dependency_lock_hash",
        72,
    )?;
    validate_sha256_hash(&dependency_lock_hash, "signature.toml artifact.dependency_lock_hash")?;
    let compiler_version =
        required_metadata_text(artifact.get("compiler_version"), "signature.toml artifact.compiler_version", 256)?;
    let language_version =
        required_metadata_text(artifact.get("language_version"), "signature.toml artifact.language_version", 64)?;

    let signing = required_toml_table(root.get("signing"), "signature.toml [signing]")?;
    reject_unknown_fields(
        signing,
        &[
            "algorithm",
            "authority",
            "signature",
            "policy_version",
            "issued_at",
            "expires_at",
            "signing_record_id",
        ],
        "signature.toml signing",
    )?;
    let algorithm = optional_metadata_text(signing.get("algorithm"), "signature.toml signing.algorithm", 32)?
        .unwrap_or_else(|| "ed25519".to_string());
    if algorithm != "ed25519" {
        return Err(TinyOneError::compile(format!(
            "Unsupported module signature algorithm {algorithm:?}; expected \"ed25519\""
        )));
    }
    let authority_id = required_metadata_text(signing.get("authority"), "signature.toml signing.authority", 128)?;
    validate_authority_id(&authority_id, "signature.toml signing.authority")?;
    let signature = signing
        .get("signature")
        .map(|value| parse_ed25519_signature(value, "signature.toml signing.signature"))
        .transpose()?;
    if signature_required && signature.is_none() {
        return Err(TinyOneError::compile("signature.toml signing.signature is required for module verification"));
    }
    let policy_version =
        required_metadata_text(signing.get("policy_version"), "signature.toml signing.policy_version", 64)?;
    let issued_at = parse_unsigned_toml(signing.get("issued_at"), "signature.toml signing.issued_at")?;
    let expires_at = parse_unsigned_toml(signing.get("expires_at"), "signature.toml signing.expires_at")?;
    let signing_record_id =
        required_metadata_text(signing.get("signing_record_id"), "signature.toml signing.signing_record_id", 256)?;
    validate_authority_id(&signing_record_id, "signature.toml signing.signing_record_id")?;

    Ok(SignatureMetadata {
        source_hash,
        binary_hash,
        dependency_lock_hash,
        compiler_version,
        language_version,
        authority_id,
        policy_version,
        issued_at,
        expires_at,
        signing_record_id,
        signature,
    })
}

fn canonical_sidecar(module_directory: &Path, filename: &str, kind: &str) -> Result<PathBuf> {
    let path = module_directory.join(filename);
    let canonical = path
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("Signed module {kind} read error: {error}")))?;
    if canonical.parent() != Some(module_directory) || canonical.file_name().is_none_or(|name| name != filename) {
        return Err(TinyOneError::compile(format!(
            "Signed module {kind} must not be a symlink outside the module directory"
        )));
    }
    Ok(canonical)
}

fn import_name_requires_manifest_match(import_path: &str) -> bool {
    let path = Path::new(import_path);
    path.components().count() == 1 && path.extension().is_none()
}

fn required_toml_table<'a>(value: Option<&'a TomlValue>, field: &str) -> Result<&'a toml::map::Map<String, TomlValue>> {
    value
        .and_then(TomlValue::as_table)
        .ok_or_else(|| TinyOneError::compile(format!("{field} must be a table")))
}

fn optional_toml_table<'a>(
    value: Option<&'a TomlValue>,
    field: &str,
) -> Result<Option<&'a toml::map::Map<String, TomlValue>>> {
    value
        .map(|value| {
            value
                .as_table()
                .ok_or_else(|| TinyOneError::compile(format!("{field} must be a table")))
        })
        .transpose()
}

fn nested_capability_table<'a>(
    capabilities: Option<&'a toml::map::Map<String, TomlValue>>,
    name: &str,
    fields: &[&str],
) -> Result<Option<&'a toml::map::Map<String, TomlValue>>> {
    let Some(capabilities) = capabilities else {
        return Ok(None);
    };
    let table = optional_toml_table(capabilities.get(name), &format!("module.toml capabilities.{name}"))?;
    if let Some(table) = table {
        reject_unknown_fields(table, fields, &format!("module.toml capabilities.{name}"))?;
    }
    Ok(table)
}

fn capability_bool(table: Option<&toml::map::Map<String, TomlValue>>, field: &str, name: &str) -> Result<bool> {
    table
        .and_then(|table| table.get(field))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| TinyOneError::compile(format!("{name} must be a boolean")))
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn parse_environment_names(value: &TomlValue, field: &str) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .ok_or_else(|| TinyOneError::compile(format!("{field} must be an array of environment variable names")))?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let name = required_metadata_text(Some(value), field, 128)?;
        if !name
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte.is_ascii_uppercase() || byte == b'_' || (index > 0 && byte.is_ascii_digit()))
        {
            return Err(TinyOneError::compile(format!("{field} must contain uppercase environment variable names")));
        }
        if result.contains(&name) {
            return Err(TinyOneError::compile(format!("{field} contains duplicate variable {name:?}")));
        }
        result.push(name);
    }
    Ok(result)
}

fn parse_dependencies(value: Option<&TomlValue>) -> Result<BTreeMap<String, String>> {
    let Some(dependencies) = value else {
        return Ok(BTreeMap::new());
    };
    let dependencies = dependencies
        .as_table()
        .ok_or_else(|| TinyOneError::compile("module.toml dependencies must be a table"))?;
    let mut parsed = BTreeMap::new();
    for (name, version) in dependencies {
        validate_authority_id(name, "module.toml dependency name")?;
        parsed.insert(
            name.clone(),
            required_metadata_text(Some(version), &format!("module.toml dependencies.{name}"), 256)?,
        );
    }
    Ok(parsed)
}

fn dependency_lock_hash(dependencies: &BTreeMap<String, String>) -> String {
    let mut payload = b"tinyone-dependency-lock-v1\0".to_vec();
    canonical_dependencies(&mut payload, dependencies);
    sha256_prefixed(&payload)
}

fn required_metadata_text(value: Option<&TomlValue>, field: &str, max_bytes: usize) -> Result<String> {
    optional_metadata_text(value, field, max_bytes)?
        .ok_or_else(|| TinyOneError::compile(format!("{field} is required")))
}

fn optional_metadata_text(value: Option<&TomlValue>, field: &str, max_bytes: usize) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| TinyOneError::compile(format!("{field} must be a string")))?;
    if value.is_empty() || value.len() > max_bytes {
        return Err(TinyOneError::compile(format!("{field} must contain between 1 and {max_bytes} UTF-8 bytes")));
    }
    Ok(Some(value.to_string()))
}

fn parse_unsigned_toml(value: Option<&TomlValue>, field: &str) -> Result<u64> {
    value
        .and_then(TomlValue::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| TinyOneError::compile(format!("{field} must be a non-negative integer")))
}

fn parse_ed25519_signature(value: &TomlValue, field: &str) -> Result<Signature> {
    let signature_hex = required_metadata_text(Some(value), field, 128)?;
    let signature_bytes =
        hex::decode(&signature_hex).map_err(|_| TinyOneError::compile(format!("{field} must be hexadecimal")))?;
    Signature::from_slice(&signature_bytes)
        .map_err(|_| TinyOneError::compile(format!("{field} must be a 64-byte Ed25519 signature")))
}

fn validate_sha256_hash(value: &str, field: &str) -> Result<()> {
    let encoded = value
        .strip_prefix("sha256:")
        .ok_or_else(|| TinyOneError::compile(format!("{field} must use the canonical sha256:<lowercase-hex> form")))?;
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(TinyOneError::compile(format!("{field} must use the canonical sha256:<lowercase-hex> form")));
    }
    Ok(())
}

fn sha256_digest(value: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(value);
    let mut output = [0u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn sha256_prefixed(value: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha256_digest(value)))
}

fn parse_modules(value: Option<&TomlValue>) -> Result<HashMap<String, ConfigModule>> {
    let Some(modules) = value else {
        return Ok(HashMap::new());
    };
    let modules = modules
        .as_table()
        .ok_or_else(|| TinyOneError::compile("Config.toml field \"modules\" must be a table"))?;
    let mut parsed = HashMap::with_capacity(modules.len());
    for (name, entry) in modules {
        let entry = entry
            .as_table()
            .ok_or_else(|| TinyOneError::compile(format!("modules.{name} must be a table")))?;
        reject_unknown_fields(entry, &["path", "permissions"], &format!("modules.{name}"))?;
        let path = required_string(entry.get("path"), &format!("modules.{name}.path"))?.to_string();
        validate_module_path(&path, &format!("modules.{name}.path"))?;
        let capabilities = entry
            .get("permissions")
            .map(|value| parse_capability_list(value, &format!("modules.{name}.permissions")))
            .transpose()?
            .unwrap_or_else(ModuleCapabilities::none);
        parsed.insert(name.clone(), ConfigModule { path, capabilities });
    }
    Ok(parsed)
}

fn parse_signing(value: Option<&TomlValue>) -> Result<SigningPolicy> {
    let Some(signing) = value else {
        return Ok(SigningPolicy::default());
    };
    let signing = signing
        .as_table()
        .ok_or_else(|| TinyOneError::compile("Config.toml field \"signing\" must be a table"))?;
    reject_unknown_fields(signing, &["require_module_signatures", "authorities"], "signing")?;
    let require_module_signatures =
        optional_bool(signing.get("require_module_signatures"), "signing.require_module_signatures")?.unwrap_or(false);
    if !require_module_signatures {
        return Ok(SigningPolicy::default());
    }
    let central_roots = central_roots()?;
    if central_roots.is_empty() {
        return Err(TinyOneError::compile(
            "Central signing is enabled but this TinyOne build has no central root. Build the release with TINYONE_CENTRAL_AUTHORITY_ROOTS=key-id=32-byte-ed25519-public-key-hex.",
        ));
    }
    let authorities = parse_authorities(signing.get("authorities"), &central_roots)?;
    Ok(SigningPolicy {
        require_module_signatures,
        authorities,
    })
}

/// Decodes the TinyOne central roots pinned into this binary at build time.
/// The release pipeline, not a project Config.toml, chooses these keys, so a
/// module publisher cannot make itself trusted by editing its source tree.
/// Multiple `id=hex` entries are comma-separated to support root rotation.
fn central_roots() -> Result<HashMap<String, VerifyingKey>> {
    let Some(roots) = CENTRAL_ROOTS_ENV else {
        return Ok(HashMap::new());
    };
    let mut parsed = HashMap::new();
    for item in roots.split(',').filter(|item| !item.is_empty()) {
        let (id, public_key_hex) = item.split_once('=').ok_or_else(|| {
            TinyOneError::compile(
                "TINYONE_CENTRAL_AUTHORITY_ROOTS entries must use key-id=32-byte-ed25519-public-key-hex",
            )
        })?;
        validate_authority_id(id, "TINYONE_CENTRAL_AUTHORITY_ROOTS key id")?;
        let public_key = parse_verifying_key(public_key_hex, "TINYONE_CENTRAL_AUTHORITY_ROOTS public key")?;
        if parsed.insert(id.to_string(), public_key).is_some() {
            return Err(TinyOneError::compile(format!(
                "TINYONE_CENTRAL_AUTHORITY_ROOTS contains duplicate root id {id:?}"
            )));
        }
    }
    Ok(parsed)
}

fn parse_authorities(
    value: Option<&TomlValue>,
    central_roots: &HashMap<String, VerifyingKey>,
) -> Result<HashMap<String, AuthorityCertificate>> {
    let Some(authorities) = value else {
        return Ok(HashMap::new());
    };
    let authorities = authorities
        .as_array()
        .ok_or_else(|| TinyOneError::compile("signing.authorities must be an array of tables"))?;
    let now = unix_time_now()?;
    let mut parsed = HashMap::with_capacity(authorities.len());
    for (index, item) in authorities.iter().enumerate() {
        let authority = item
            .as_table()
            .ok_or_else(|| TinyOneError::compile(format!("signing.authorities[{index}] must be a table")))?;
        reject_unknown_fields(
            authority,
            &["id", "issuer", "public_key", "not_before", "expires", "certificate"],
            "signing.authorities entry",
        )?;
        let id = required_string(authority.get("id"), "signing.authorities.id")?;
        validate_authority_id(id, "signing.authorities.id")?;
        let issuer = required_string(authority.get("issuer"), "signing.authorities.issuer")?;
        let issuer_key = central_roots.get(issuer).ok_or_else(|| {
            TinyOneError::compile(format!("Authority {id:?} names unknown central issuer {issuer:?}"))
        })?;
        let public_key_hex = required_string(authority.get("public_key"), "signing.authorities.public_key")?;
        let public_key = parse_verifying_key(public_key_hex, "signing.authorities.public_key")?;
        let not_before = parse_u64(authority.get("not_before"), "signing.authorities.not_before")?;
        let expires = parse_u64(authority.get("expires"), "signing.authorities.expires")?;
        if expires <= not_before {
            return Err(TinyOneError::compile(format!("Authority {id:?} expiry must be after not_before")));
        }
        if now < not_before || now > expires {
            return Err(TinyOneError::compile(format!("Authority {id:?} certificate is not currently valid")));
        }
        let certificate_hex = required_string(authority.get("certificate"), "signing.authorities.certificate")?;
        let certificate_bytes = hex::decode(certificate_hex)
            .map_err(|_| TinyOneError::compile("signing.authorities.certificate must be hexadecimal"))?;
        let certificate = Signature::from_slice(&certificate_bytes).map_err(|_| {
            TinyOneError::compile("signing.authorities.certificate must be a 64-byte Ed25519 signature")
        })?;
        let digest = authority_certificate_digest(id, public_key.as_bytes(), not_before, expires)?;
        issuer_key.verify(&digest, &certificate).map_err(|_| {
            TinyOneError::compile(format!("Authority {id:?} certificate failed central-root verification"))
        })?;
        if parsed
            .insert(id.to_string(), AuthorityCertificate { public_key })
            .is_some()
        {
            return Err(TinyOneError::compile(format!("signing.authorities contains duplicate authority id {id:?}")));
        }
    }
    Ok(parsed)
}

/// Canonical certificate payload used to delegate module-signing authority to
/// a company. Central roots sign the SHA-256 digest returned by
/// [`authority_certificate_digest`], not this byte representation directly.
pub fn authority_certificate_payload(
    authority_id: &str,
    public_key: &[u8; 32],
    not_before: u64,
    expires: u64,
) -> Result<Vec<u8>> {
    validate_authority_id(authority_id, "authority certificate id")?;
    if expires <= not_before {
        return Err(TinyOneError::compile("authority certificate expiry must be after not_before"));
    }
    let mut payload = Vec::with_capacity(48 + authority_id.len());
    payload.extend_from_slice(b"tinyone-authority-certificate-v1\0");
    payload.extend_from_slice(authority_id.as_bytes());
    payload.push(0);
    payload.extend_from_slice(public_key);
    payload.extend_from_slice(&not_before.to_le_bytes());
    payload.extend_from_slice(&expires.to_le_bytes());
    Ok(payload)
}

/// SHA-256 digest signed by a TinyOne central root for an authority
/// delegation certificate. The root private key remains exclusively in the
/// central authority's signing service.
pub fn authority_certificate_digest(
    authority_id: &str,
    public_key: &[u8; 32],
    not_before: u64,
    expires: u64,
) -> Result<[u8; 32]> {
    Ok(sha256_digest(&authority_certificate_payload(authority_id, public_key, not_before, expires)?))
}

fn validate_authority_id(id: &str, field: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(TinyOneError::compile(format!(
            "{field} must contain 1 through 128 ASCII letters, digits, '.', '_', or '-'"
        )));
    }
    Ok(())
}

fn parse_verifying_key(value: &str, field: &str) -> Result<VerifyingKey> {
    let public_key_bytes =
        hex::decode(value).map_err(|_| TinyOneError::compile(format!("{field} must be hexadecimal")))?;
    let public_key: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| TinyOneError::compile(format!("{field} must be a 32-byte Ed25519 public key")))?;
    VerifyingKey::from_bytes(&public_key)
        .map_err(|_| TinyOneError::compile(format!("{field} is not a valid Ed25519 key")))
}

fn parse_u64(value: Option<&TomlValue>, field: &str) -> Result<u64> {
    value
        .and_then(TomlValue::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a non-negative integer")))
}

fn unix_time_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| TinyOneError::compile("System clock is before the Unix epoch"))
}

fn validate_package(value: Option<&TomlValue>) -> Result<()> {
    let Some(package) = value else {
        return Ok(());
    };
    let package = package
        .as_table()
        .ok_or_else(|| TinyOneError::compile("Config.toml field \"package\" must be a table"))?;
    reject_unknown_fields(package, &["name", "version", "description", "author", "authors"], "package")?;
    for field in ["name", "version", "description", "author"] {
        let _ = optional_string(package.get(field), &format!("package.{field}"))?;
    }
    if let Some(authors) = package.get("authors") {
        let authors = authors
            .as_array()
            .ok_or_else(|| TinyOneError::compile("package.authors must be an array of strings"))?;
        for author in authors {
            if author.as_str().is_none() {
                return Err(TinyOneError::compile("package.authors must be an array of strings"));
            }
        }
    }
    Ok(())
}

fn validate_module_path(path: &str, field: &str) -> Result<()> {
    let target = Path::new(path);
    if target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || !target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("to"))
    {
        return Err(TinyOneError::compile(format!("{field} must be a non-escaping TinyLang .to source path")));
    }
    Ok(())
}

fn parse_capability_list(value: &TomlValue, field: &str) -> Result<ModuleCapabilities> {
    let values = value
        .as_array()
        .ok_or_else(|| TinyOneError::compile(format!("{field} must be an array of capability names")))?;
    let names = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| TinyOneError::compile(format!("{field} must contain only strings")))
        })
        .collect::<Result<Vec<_>>>()?;
    ModuleCapabilities::from_names(&names)
}

fn optional_table<'a>(
    value: Option<&'a TomlValue>,
    field: &str,
) -> Result<Option<&'a toml::map::Map<String, TomlValue>>> {
    value
        .map(|value| {
            value
                .as_table()
                .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a table")))
        })
        .transpose()
}

fn reject_unknown_fields(table: &toml::map::Map<String, TomlValue>, allowed: &[&str], scope: &str) -> Result<()> {
    if let Some(field) = table.keys().find(|field| !allowed.contains(&field.as_str())) {
        return Err(TinyOneError::compile(format!("Unknown field {scope}.{field}")));
    }
    Ok(())
}

fn required_string<'a>(value: Option<&'a TomlValue>, field: &str) -> Result<&'a str> {
    value
        .and_then(TomlValue::as_str)
        .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a string")))
}

fn optional_string<'a>(value: Option<&'a TomlValue>, field: &str) -> Result<Option<&'a str>> {
    value
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a string")))
        })
        .transpose()
}

fn optional_bool(value: Option<&TomlValue>, field: &str) -> Result<Option<bool>> {
    value
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a boolean")))
        })
        .transpose()
}

fn parse_usize(value: &TomlValue, field: &str) -> Result<usize> {
    value
        .as_integer()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| TinyOneError::compile(format!("Config.toml field {field:?} must be a non-negative integer")))
}

fn read_limited(path: &Path, max_bytes: usize, kind: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(path).map_err(|error| TinyOneError::compile(format!("{kind} read error: {error}")))?;
    if bytes.len() > max_bytes {
        return Err(TinyOneError::compile(format!("{kind} rejected: byte size limit {max_bytes} exceeded")));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn signed_manifest_runtime_permissions_keep_fine_grained_grants() {
        let manifest = parse_module_manifest(
            br#"
            [module]
            name = "fine-policy"
            version = "1.0.0"
            publisher = "example-corp"

            [purpose]
            description = "Tests exact runtime permissions"

            [capabilities.filesystem]
            read = true
            write = false

            [capabilities.environment]
            read = ["HTTP_PROXY"]

            [capabilities.network]
            outbound = true
            listen = false

            [capabilities.process]
            spawn = true
            "#,
        )
        .expect("valid manifest");
        let permissions = manifest.runtime_permissions();
        assert!(permissions.allows_filesystem_read());
        assert!(!permissions.allows_filesystem_write());
        assert!(permissions.allows_environment_read("HTTP_PROXY"));
        assert!(!permissions.allows_environment_read("SECRET_TOKEN"));
        assert!(permissions.network_outbound());
        assert!(!permissions.network_listen());
        assert!(permissions.process_spawn());
        assert!(!permissions.linux_pipelines_allowed());
    }

    #[test]
    fn central_root_certificate_delegates_company_module_signing() {
        let central = SigningKey::from_bytes(&[31u8; 32]);
        let company = SigningKey::from_bytes(&[47u8; 32]);
        let authority_id = "example-corp";
        let not_before = unix_time_now().expect("clock").saturating_sub(1);
        let expires = not_before + 3_600;
        let certificate = central.sign(
            &authority_certificate_digest(authority_id, company.verifying_key().as_bytes(), not_before, expires)
                .expect("certificate digest"),
        );
        let config: TomlValue = format!(
            r#"
            [signing]
            [[signing.authorities]]
            id = "{authority_id}"
            issuer = "tinyone-root-test"
            public_key = "{}"
            not_before = {not_before}
            expires = {expires}
            certificate = "{}"
            "#,
            hex::encode(company.verifying_key().as_bytes()),
            hex::encode(certificate.to_bytes()),
        )
        .parse()
        .expect("authority configuration");
        let mut roots = HashMap::new();
        roots.insert("tinyone-root-test".to_string(), central.verifying_key());
        let authorities = parse_authorities(
            config
                .get("signing")
                .and_then(TomlValue::as_table)
                .and_then(|table| table.get("authorities")),
            &roots,
        )
        .expect("central certificate verifies");

        let root = std::env::temp_dir().join(format!(
            "tinyone-signed-module-{}-{}",
            std::process::id(),
            unix_time_now().expect("clock")
        ));
        fs::create_dir_all(&root).expect("test directory");
        let module_path = root.join("example.to");
        let source = "export fn value() { return 1 }\n";
        fs::write(&module_path, source).expect("module source");
        let module_path = module_path.canonicalize().expect("canonical module source");
        let manifest = r#"
            [module]
            name = "example-module"
            version = "3.2.1"
            publisher = "example-corp"

            [purpose]
            description = "A small signed example"

            [capabilities.network]
            outbound = true
            listen = false

            [dependencies]
            url-parser = "1.8.3"
        "#;
        let manifest_data = parse_module_manifest(manifest.as_bytes()).expect("manifest");
        let source_hash = sha256_prefixed(source.as_bytes());
        let dependency_lock_hash = dependency_lock_hash(&manifest_data.dependencies);
        let issued_at = unix_time_now().expect("clock").saturating_sub(1);
        let expires_at = issued_at + 600;
        let unsigned_signature = format!(
            r#"
                [artifact]
                source_hash = "{source_hash}"
                dependency_lock_hash = "{dependency_lock_hash}"
                compiler_version = "{}"
                language_version = "{TINYONE_LANGUAGE_VERSION}"

                [signing]
                authority = "{authority_id}"
                policy_version = "{MODULE_SIGNATURE_POLICY_VERSION}"
                issued_at = {issued_at}
                expires_at = {expires_at}
                signing_record_id = "example-signing-record-1"
            "#,
            env!("CARGO_PKG_VERSION"),
        );
        let digest = module_signature_digest(manifest, &unsigned_signature).expect("module digest");
        let module_signature = company.sign(&digest);
        let signature = unsigned_signature.replacen(
            "signing_record_id = \"example-signing-record-1\"",
            &format!(
                "signing_record_id = \"example-signing-record-1\"\nsignature = \"{}\"",
                hex::encode(module_signature.to_bytes())
            ),
            1,
        );
        fs::write(root.join(MODULE_MANIFEST_FILE), manifest).expect("module manifest");
        fs::write(root.join(MODULE_SIGNATURE_FILE), signature).expect("signature manifest");

        let config = ProjectConfig {
            project_root:               root.clone(),
            sandbox_enabled:            true,
            require_configured_modules: false,
            root_capabilities:          ModuleCapabilities::all(),
            vm_settings:                VmSettings::default(),
            modules:                    HashMap::new(),
            signing:                    SigningPolicy {
                require_module_signatures: true,
                authorities,
            },
            input:                      None,
        };
        let verified = config
            .verify_module_signature("example-module", &module_path, source)
            .expect("signature verifies")
            .expect("signature required");
        assert!(verified.declared_capabilities.allows(crate::ModuleCapability::Network));
        fs::remove_dir_all(root).expect("test directory cleanup");
    }

    #[test]
    fn direct_project_trusted_keys_are_not_a_signing_escape_hatch() {
        let config: TomlValue = r#"
            [signing]
            [[signing.trusted_keys]]
            id = "self"
            public_key = "0000000000000000000000000000000000000000000000000000000000000000"
            "#
        .parse()
        .expect("configuration syntax");
        let signing = config.get("signing");
        let error = parse_signing(signing).expect_err("direct trust must be rejected");
        assert!(error.to_string().contains("signing.trusted_keys"), "{error}");
    }
}
