use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use blake2::{Blake2b512, Digest};
use serde::{Deserialize, Serialize};

use crate::{
    ModuleResolver,
    ResolverInput,
    Result,
    TinyOneError,
    VerifiedProgram,
    content_digest,
    load_verified_artifact,
    write_binary_artifact,
};

// v8 records the earliest signed-module expiry.  A cache entry must never
// authorize a module after the signature used to compile it has expired.
const CACHE_FORMAT_VERSION: u32 = 8;
const MAX_CACHE_METADATA_BYTES: usize = 1024 * 1024;
const MAX_CACHE_INPUT_BYTES: usize = 1024 * 1024;
const SMALL_CACHE_MAX_MODULES: usize = 2;
const SMALL_CACHE_MAX_SOURCE_BYTES: usize = 4 * 1024;
const WINDOWS_MEDIUM_CACHE_MAX_MODULES: usize = 16;
const WINDOWS_MEDIUM_CACHE_MAX_SOURCE_BYTES: usize = 64 * 1024;
const BYPASS_MEMORY_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileCacheStatus {
    /// A dependency-validated binary artifact was loaded.
    Hit,
    /// One changed module was rebuilt and patched into a validated artifact.
    Incremental,
    /// No reusable cache entry existed, so a new entry was written after compilation.
    Miss,
    /// The graph was compiled directly because cache validation would cost more than rebuilding it.
    Bypassed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheRecord {
    format_version:       u32,
    compiler_version:     String,
    optimize:             bool,
    root:                 PathBuf,
    fingerprint:          String,
    module_count:         usize,
    input_bytes:          usize,
    /// Earliest expiry among module signatures verified during compilation.
    /// `None` means the source graph did not require module signatures.
    signature_expires_at: Option<u64>,
    inputs:               Vec<CacheInput>,
    resolutions:          Vec<CacheResolution>,
    modules:              Vec<CacheModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheInput {
    path:     PathBuf,
    digest:   Option<String>,
    identity: Option<CacheInputIdentity>,
}

/// A cheap filesystem identity used only to detect a definitely stale record.
/// A matching identity never authorizes a cache hit; content digests remain the
/// trust decision because timestamps and lengths can be preserved or spoofed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheInputIdentity {
    size_bytes:          u64,
    modified_unix_nanos: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheResolution {
    candidate: PathBuf,
    canonical: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheModule {
    path: PathBuf,
    name: String,
}

pub(crate) struct IncrementalCandidate {
    pub(crate) program:       Option<VerifiedProgram>,
    pub(crate) module_path:   PathBuf,
    pub(crate) module_name:   String,
    pub(crate) module_source: Option<String>,
    pub(crate) module_digest: [u8; 16],
    record:                   CacheRecord,
}

pub(crate) enum CacheLookup {
    Hit(VerifiedProgram),
    Incremental(Box<IncrementalCandidate>),
    Bypass,
    Miss,
}

pub(crate) fn lookup(root: &Path, optimize: bool) -> Result<CacheLookup> {
    let paths = cache_paths(root, optimize);
    let Some(record) = load_record(&paths.metadata) else {
        return Ok(CacheLookup::Miss);
    };
    if !record_matches_request(&record, root, optimize) {
        return Ok(CacheLookup::Miss);
    }
    if !signature_cache_entry_is_current(record.signature_expires_at)? {
        return Ok(CacheLookup::Miss);
    }
    if should_bypass_shape(record.module_count, record.input_bytes) {
        return Ok(CacheLookup::Bypass);
    }

    // This prefilter may only reject a record. A matching identity deliberately
    // falls through to full digest validation below.
    let metadata_changed = record
        .inputs
        .iter()
        .filter(|input| input_metadata_match(input) == InputMetadataMatch::Changed)
        .collect::<Vec<_>>();
    if metadata_changed.len() > 1
        || metadata_changed
            .first()
            .is_some_and(|input| input.path == root || input.digest.is_none())
    {
        return Ok(CacheLookup::Miss);
    }

    let mut changed = None;
    for input in &record.inputs {
        let state = match_input(input);
        if let InputMatch::Changed { bytes, digest } = state {
            if changed.is_some() {
                return Ok(CacheLookup::Miss);
            }
            changed = Some((input.path.clone(), input.digest.is_some(), bytes, digest));
        }
    }
    if changed
        .as_ref()
        .is_some_and(|(path, existed, bytes, digest)| path == root || !existed || bytes.is_none() || digest.is_none())
    {
        return Ok(CacheLookup::Miss);
    }
    if !resolutions_match(&record.resolutions) {
        return Ok(CacheLookup::Miss);
    }

    let Some((path, existed, bytes, digest)) = changed else {
        return Ok(match_cached_program(&paths.artifact, &record.fingerprint)
            .map(CacheLookup::Hit)
            .unwrap_or(CacheLookup::Miss));
    };
    debug_assert!(path != root && existed);
    let (Some(bytes), Some(digest)) = (bytes, digest) else {
        return Ok(CacheLookup::Miss);
    };
    let Ok(source) = String::from_utf8(bytes) else {
        return Ok(CacheLookup::Miss);
    };
    let Some(module) = record.modules.iter().find(|module| module.path == path) else {
        return Ok(CacheLookup::Miss);
    };
    let module_name = module.name.clone();
    let Some(verified) = match_cached_program(&paths.artifact, &record.fingerprint) else {
        return Ok(CacheLookup::Miss);
    };
    Ok(CacheLookup::Incremental(Box::new(IncrementalCandidate {
        program: Some(verified),
        module_path: path,
        module_name,
        module_source: Some(source),
        module_digest: digest,
        record,
    })))
}

fn match_cached_program(path: &Path, expected_fingerprint: &str) -> Option<VerifiedProgram> {
    // Cache files live beside project sources and are not an authenticated
    // authority store. Decode them as untrusted data; any entry that dispatches
    // a builtin is rebuilt from current source/configuration rather than
    // allowed to re-grant serialized policy.
    let program = load_verified_artifact(path).ok()?;
    (!program.program().needs_runtime_host_permissions() && program.fingerprint() == expected_fingerprint)
        .then_some(program)
}

pub(crate) fn should_bypass(root_source: &str, resolver: &ModuleResolver) -> bool {
    resolver.has_capability_grants()
        || should_bypass_shape(
            resolver.module_count(),
            root_source.len().saturating_add(resolver.existing_input_bytes()),
        )
}

fn should_bypass_shape(module_count: usize, input_bytes: usize) -> bool {
    (module_count <= SMALL_CACHE_MAX_MODULES && input_bytes <= SMALL_CACHE_MAX_SOURCE_BYTES)
        || (cfg!(windows)
            && (3..=WINDOWS_MEDIUM_CACHE_MAX_MODULES).contains(&module_count)
            && input_bytes <= WINDOWS_MEDIUM_CACHE_MAX_SOURCE_BYTES)
}

pub(crate) fn should_bypass_filesystem(root: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        static IS_WSL: OnceLock<bool> = OnceLock::new();
        let is_wsl = *IS_WSL.get_or_init(|| {
            fs::read_to_string("/proc/sys/kernel/osrelease")
                .or_else(|_| fs::read_to_string("/proc/version"))
                .map(|text| text.to_ascii_lowercase().contains("microsoft"))
                .unwrap_or(false)
        });
        if is_wsl {
            let path = root.to_string_lossy();
            if let Some(suffix) = path.strip_prefix("/mnt/") {
                let bytes = suffix.as_bytes();
                return bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b'/');
            }
        }
    }
    let _ = root;
    false
}

pub(crate) fn is_known_bypass(root: &Path, optimize: bool) -> bool {
    let mut entries = bypass_entries()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(index) = entries
        .iter()
        .position(|entry| entry.root == root && entry.optimize == optimize)
    else {
        return false;
    };
    let entry = entries.remove(index).expect("known bypass entry");
    entries.push_back(entry);
    true
}

pub(crate) fn remember_bypass(root: &Path, optimize: bool) {
    let mut entries = bypass_entries()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.root == root && entry.optimize == optimize)
    {
        entries.remove(index);
    }
    entries.push_back(BypassEntry {
        root: root.to_path_buf(),
        optimize,
    });
    while entries.len() > BYPASS_MEMORY_LIMIT {
        entries.pop_front();
    }
}

#[derive(Debug)]
struct BypassEntry {
    root:     PathBuf,
    optimize: bool,
}

fn bypass_entries() -> &'static Mutex<VecDeque<BypassEntry>> {
    static ENTRIES: OnceLock<Mutex<VecDeque<BypassEntry>>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(crate) fn store(
    root: &Path,
    root_source: &str,
    optimize: bool,
    verified: &VerifiedProgram,
    resolver: &ModuleResolver,
) -> Result<()> {
    let paths = cache_paths(root, optimize);
    let mut inputs = BTreeMap::new();
    inputs.insert(root.to_path_buf(), Some(content_digest(root_source.as_bytes())));
    for ResolverInput { path, digest } in resolver.inputs() {
        inputs.insert(path, digest);
    }
    let record = CacheRecord {
        format_version: CACHE_FORMAT_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        optimize,
        root: root.to_path_buf(),
        fingerprint: verified.fingerprint().to_string(),
        module_count: resolver.module_count(),
        input_bytes: root_source.len().saturating_add(resolver.existing_input_bytes()),
        signature_expires_at: resolver.signed_module_valid_until(),
        inputs: inputs
            .into_iter()
            .map(|(path, digest)| {
                CacheInput {
                    identity: digest.as_ref().and_then(|_| cache_input_identity(&path)),
                    path,
                    digest: digest.map(hex::encode),
                }
            })
            .collect(),
        resolutions: resolver
            .canonical_resolutions()
            .into_iter()
            .map(|(candidate, canonical)| CacheResolution { candidate, canonical })
            .collect(),
        modules: resolver
            .module_names()
            .into_iter()
            .map(|(path, name)| CacheModule { path, name })
            .collect(),
    };
    store_record(&paths, verified, &record)
}

pub(crate) fn store_incremental(
    root: &Path,
    optimize: bool,
    verified: &VerifiedProgram,
    mut candidate: IncrementalCandidate,
    resolver: &ModuleResolver,
) -> Result<()> {
    let paths = cache_paths(root, optimize);
    let mut inputs = candidate
        .record
        .inputs
        .into_iter()
        .map(|input| (input.path.clone(), input))
        .collect::<BTreeMap<_, _>>();
    for ResolverInput { path, digest } in resolver.inputs() {
        inputs.insert(
            path.clone(),
            CacheInput {
                identity: digest.as_ref().and_then(|_| cache_input_identity(&path)),
                path,
                digest: digest.map(hex::encode),
            },
        );
    }
    inputs.insert(
        candidate.module_path.clone(),
        CacheInput {
            identity: cache_input_identity(&candidate.module_path),
            path:     candidate.module_path.clone(),
            digest:   Some(hex::encode(candidate.module_digest)),
        },
    );
    candidate.record.inputs = inputs.into_values().collect();
    candidate.record.fingerprint = verified.fingerprint().to_string();
    candidate.record.signature_expires_at =
        earliest_expiry(candidate.record.signature_expires_at, resolver.signed_module_valid_until());

    let mut resolutions = candidate
        .record
        .resolutions
        .into_iter()
        .map(|item| (item.candidate.clone(), item))
        .collect::<BTreeMap<_, _>>();
    for (candidate_path, canonical) in resolver.canonical_resolutions() {
        resolutions.insert(
            candidate_path.clone(),
            CacheResolution {
                candidate: candidate_path,
                canonical,
            },
        );
    }
    candidate.record.resolutions = resolutions.into_values().collect();

    let mut modules = candidate
        .record
        .modules
        .into_iter()
        .map(|item| (item.path.clone(), item))
        .collect::<BTreeMap<_, _>>();
    for (path, name) in resolver.module_names() {
        modules.insert(path.clone(), CacheModule { path, name });
    }
    candidate.record.modules = modules.into_values().collect();
    candidate.record.module_count = candidate.record.modules.len();
    candidate.record.input_bytes = candidate
        .record
        .inputs
        .iter()
        .filter_map(|input| input.identity.as_ref().map(|identity| identity.size_bytes))
        .fold(0_usize, |total, bytes| total.saturating_add(usize::try_from(bytes).unwrap_or(usize::MAX)));
    store_record(&paths, verified, &candidate.record)
}

fn store_record(paths: &CachePaths, verified: &VerifiedProgram, record: &CacheRecord) -> Result<()> {
    fs::create_dir_all(&paths.directory)
        .map_err(|error| TinyOneError::compile(format!("Compile cache directory error: {error}")))?;
    write_binary_artifact(verified.program(), &paths.artifact)?;
    let metadata = serde_json::to_vec(record)
        .map_err(|error| TinyOneError::compile(format!("Compile cache metadata error: {error}")))?;
    fs::write(&paths.metadata, metadata)
        .map_err(|error| TinyOneError::compile(format!("Compile cache write error: {error}")))
}

enum InputMatch {
    Same,
    Changed {
        bytes:  Option<Vec<u8>>,
        digest: Option<[u8; 16]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMetadataMatch {
    Same,
    Changed,
    Unavailable,
}

fn input_metadata_match(input: &CacheInput) -> InputMetadataMatch {
    match (&input.digest, &input.identity) {
        (Some(_), Some(expected)) => {
            match cache_input_identity(&input.path) {
                Some(actual) if actual == *expected => InputMetadataMatch::Same,
                Some(_) => InputMetadataMatch::Changed,
                None => InputMetadataMatch::Unavailable,
            }
        }
        (Some(_), None) => InputMetadataMatch::Unavailable,
        (None, _) if input.path.exists() => InputMetadataMatch::Changed,
        (None, _) => InputMetadataMatch::Same,
    }
}

fn cache_input_identity(path: &Path) -> Option<CacheInputIdentity> {
    let metadata = fs::metadata(path).ok()?;
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok());
    Some(CacheInputIdentity {
        size_bytes: metadata.len(),
        modified_unix_nanos,
    })
}

fn match_input(input: &CacheInput) -> InputMatch {
    match &input.digest {
        Some(expected) => {
            let Ok(bytes) = read_bounded(&input.path, MAX_CACHE_INPUT_BYTES) else {
                return InputMatch::Changed {
                    bytes:  None,
                    digest: None,
                };
            };
            let digest = content_digest(&bytes);
            if digest_matches(expected, digest) {
                InputMatch::Same
            } else {
                InputMatch::Changed {
                    bytes:  Some(bytes),
                    digest: Some(digest),
                }
            }
        }
        None if input.path.exists() => {
            InputMatch::Changed {
                bytes:  None,
                digest: None,
            }
        }
        None => InputMatch::Same,
    }
}

fn digest_matches(expected: &str, actual: [u8; 16]) -> bool {
    let mut decoded = [0_u8; 16];
    hex::decode_to_slice(expected, &mut decoded).is_ok() && decoded == actual
}

fn resolutions_match(resolutions: &[CacheResolution]) -> bool {
    resolutions.iter().all(|resolution| {
        resolution
            .candidate
            .canonicalize()
            .map(|path| path == resolution.canonical)
            .unwrap_or(false)
    })
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let metadata =
        fs::metadata(path).map_err(|error| TinyOneError::compile(format!("Compile cache read error: {error}")))?;
    if metadata.len() > max_bytes as u64 {
        return Err(TinyOneError::compile(format!("Compile cache input exceeds byte size limit {max_bytes}")));
    }
    fs::read(path).map_err(|error| TinyOneError::compile(format!("Compile cache read error: {error}")))
}

fn load_record(path: &Path) -> Option<CacheRecord> {
    let metadata = read_bounded(path, MAX_CACHE_METADATA_BYTES).ok()?;
    serde_json::from_slice(&metadata).ok()
}

fn record_matches_request(record: &CacheRecord, root: &Path, optimize: bool) -> bool {
    record.format_version == CACHE_FORMAT_VERSION
        && record.compiler_version == env!("CARGO_PKG_VERSION")
        && record.optimize == optimize
        && record.root == root
}

fn earliest_expiry(existing: Option<u64>, replacement: Option<u64>) -> Option<u64> {
    match (existing, replacement) {
        (Some(existing), Some(replacement)) => Some(existing.min(replacement)),
        (Some(existing), None) => Some(existing),
        (None, Some(replacement)) => Some(replacement),
        (None, None) => None,
    }
}

fn signature_cache_entry_is_current(expires_at: Option<u64>) -> Result<bool> {
    let Some(expires_at) = expires_at else {
        return Ok(true);
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| TinyOneError::compile("System clock is before the Unix epoch"))?;
    Ok(now <= expires_at)
}

struct CachePaths {
    directory: PathBuf,
    metadata:  PathBuf,
    artifact:  PathBuf,
}

fn cache_paths(root: &Path, optimize: bool) -> CachePaths {
    let directory = root.parent().unwrap_or_else(|| Path::new(".")).join(".tinyone-cache");
    let mut hasher = Blake2b512::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hasher.update([optimize as u8]);
    hasher.update(CACHE_FORMAT_VERSION.to_le_bytes());
    let digest = hasher.finalize();
    let key = hex::encode(&digest[..12]);
    CachePaths {
        metadata: directory.join(format!("{key}.json")),
        artifact: directory.join(format!("{key}.tob")),
        directory,
    }
}

#[cfg(test)]
mod tests {
    use super::{earliest_expiry, signature_cache_entry_is_current};

    #[test]
    fn signature_expiry_invalidates_cache_entries_after_expiry() {
        assert!(signature_cache_entry_is_current(None).expect("unsigned cache entry"));
        assert!(signature_cache_entry_is_current(Some(u64::MAX)).expect("future signature"));
        assert!(!signature_cache_entry_is_current(Some(0)).expect("expired signature"));
        assert_eq!(earliest_expiry(Some(30), Some(20)), Some(20));
        assert_eq!(earliest_expiry(Some(30), None), Some(30));
    }
}
