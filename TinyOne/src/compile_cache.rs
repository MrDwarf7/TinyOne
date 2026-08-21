use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use blake2::{Blake2b512, Digest};
use serde::{Deserialize, Serialize};

use crate::{
    ModuleResolver, ResolverInput, Result, TinyOneError, VerifiedProgram, content_digest,
    load_verified_artifact, write_binary_artifact,
};

const CACHE_FORMAT_VERSION: u32 = 2;
const MAX_CACHE_METADATA_BYTES: usize = 1024 * 1024;
const MAX_CACHE_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileCacheStatus {
    Hit,
    Incremental,
    Miss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheRecord {
    format_version: u32,
    compiler_version: String,
    optimize: bool,
    root: PathBuf,
    fingerprint: String,
    inputs: Vec<CacheInput>,
    resolutions: Vec<CacheResolution>,
    modules: Vec<CacheModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheInput {
    path: PathBuf,
    digest: Option<String>,
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
    pub(crate) program: Option<VerifiedProgram>,
    pub(crate) module_path: PathBuf,
    pub(crate) module_name: String,
    record: CacheRecord,
}

pub(crate) fn try_load(root: &Path, optimize: bool) -> Result<Option<VerifiedProgram>> {
    let paths = cache_paths(root, optimize);
    let Some(record) = load_record(&paths.metadata) else {
        return Ok(None);
    };
    if !record_matches_request(&record, root, optimize)
        || !inputs_match(&record.inputs)
        || !resolutions_match(&record.resolutions)
    {
        return Ok(None);
    }
    let verified = match load_verified_artifact(&paths.artifact) {
        Ok(program) => program,
        Err(_) => return Ok(None),
    };
    if verified.fingerprint() != record.fingerprint {
        return Ok(None);
    }
    Ok(Some(verified))
}

pub(crate) fn try_load_incremental(
    root: &Path,
    optimize: bool,
) -> Result<Option<IncrementalCandidate>> {
    let paths = cache_paths(root, optimize);
    let Some(record) = load_record(&paths.metadata) else {
        return Ok(None);
    };
    if !record_matches_request(&record, root, optimize) || !resolutions_match(&record.resolutions) {
        return Ok(None);
    }
    let changed = record
        .inputs
        .iter()
        .filter(|input| !input_matches(input))
        .collect::<Vec<_>>();
    if changed.len() != 1 || changed[0].path == root || changed[0].digest.is_none() {
        return Ok(None);
    }
    let Some(module) = record
        .modules
        .iter()
        .find(|module| module.path == changed[0].path)
    else {
        return Ok(None);
    };
    let program = match load_verified_artifact(&paths.artifact) {
        Ok(program) if program.fingerprint() == record.fingerprint => program,
        _ => return Ok(None),
    };
    Ok(Some(IncrementalCandidate {
        program: Some(program),
        module_path: module.path.clone(),
        module_name: module.name.clone(),
        record,
    }))
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
    inputs.insert(
        root.to_path_buf(),
        Some(content_digest(root_source.as_bytes())),
    );
    for ResolverInput { path, digest } in resolver.inputs() {
        inputs.insert(path, digest);
    }
    let record = CacheRecord {
        format_version: CACHE_FORMAT_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        optimize,
        root: root.to_path_buf(),
        fingerprint: verified.fingerprint().to_string(),
        inputs: inputs
            .into_iter()
            .map(|(path, digest)| CacheInput {
                path,
                digest: digest.map(hex::encode),
            })
            .collect(),
        resolutions: resolver
            .canonical_resolutions()
            .into_iter()
            .map(|(candidate, canonical)| CacheResolution {
                candidate,
                canonical,
            })
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
                path,
                digest: digest.map(hex::encode),
            },
        );
    }
    let current = read_bounded(&candidate.module_path, MAX_CACHE_INPUT_BYTES)?;
    inputs.insert(
        candidate.module_path.clone(),
        CacheInput {
            path: candidate.module_path.clone(),
            digest: Some(hex::encode(content_digest(&current))),
        },
    );
    candidate.record.inputs = inputs.into_values().collect();
    candidate.record.fingerprint = verified.fingerprint().to_string();

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
    store_record(&paths, verified, &candidate.record)
}

fn store_record(
    paths: &CachePaths,
    verified: &VerifiedProgram,
    record: &CacheRecord,
) -> Result<()> {
    fs::create_dir_all(&paths.directory).map_err(|error| {
        TinyOneError::compile(format!("Compile cache directory error: {error}"))
    })?;
    write_binary_artifact(verified.program(), &paths.artifact)?;
    let metadata = serde_json::to_vec(record)
        .map_err(|error| TinyOneError::compile(format!("Compile cache metadata error: {error}")))?;
    fs::write(&paths.metadata, metadata)
        .map_err(|error| TinyOneError::compile(format!("Compile cache write error: {error}")))
}

fn inputs_match(inputs: &[CacheInput]) -> bool {
    inputs.iter().all(input_matches)
}

fn input_matches(input: &CacheInput) -> bool {
    match &input.digest {
        Some(expected) => read_bounded(&input.path, MAX_CACHE_INPUT_BYTES)
            .map(|bytes| hex::encode(content_digest(&bytes)) == *expected)
            .unwrap_or(false),
        None => !input.path.exists(),
    }
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
    let metadata = fs::metadata(path)
        .map_err(|error| TinyOneError::compile(format!("Compile cache read error: {error}")))?;
    if metadata.len() > max_bytes as u64 {
        return Err(TinyOneError::compile(format!(
            "Compile cache input exceeds byte size limit {max_bytes}"
        )));
    }
    fs::read(path)
        .map_err(|error| TinyOneError::compile(format!("Compile cache read error: {error}")))
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

struct CachePaths {
    directory: PathBuf,
    metadata: PathBuf,
    artifact: PathBuf,
}

fn cache_paths(root: &Path, optimize: bool) -> CachePaths {
    let directory = root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".tinyone-cache");
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
