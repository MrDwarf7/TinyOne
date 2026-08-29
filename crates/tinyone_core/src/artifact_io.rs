use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::bytecode::artifact::MAX_ARTIFACT_BYTES;
use crate::bytecode::binary::{BINARY_ARTIFACT_MAGIC, MAX_BINARY_ARTIFACT_BYTES};
use crate::{Program, Result, TinyOneError, VerifiedProgram};

pub fn load_artifact(path: impl AsRef<Path>) -> Result<Program> {
    load_verified_artifact(path).map(VerifiedProgram::into_program)
}

pub fn load_verified_artifact(path: impl AsRef<Path>) -> Result<VerifiedProgram> {
    load_verified_untrusted_artifact(path.as_ref())
}

fn load_verified_untrusted_artifact(path: &Path) -> Result<VerifiedProgram> {
    let bytes = read_limited_artifact(path)?;
    if bytes.starts_with(BINARY_ARTIFACT_MAGIC) {
        return VerifiedProgram::from_binary_artifact(&bytes);
    }
    let text =
        String::from_utf8(bytes).map_err(|error| TinyOneError::compile(format!("Artifact must be UTF-8: {error}")))?;
    let data =
        serde_json::from_str(&text).map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    VerifiedProgram::from_artifact(data)
}

pub fn write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let text = serde_json::to_string_pretty(&program.to_artifact())
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::compile(format!("Artifact write error: {error}")))
}

pub fn write_binary_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let bytes = program.to_binary_artifact()?;
    fs::write(path, bytes).map_err(|error| TinyOneError::compile(format!("Binary artifact write error: {error}")))
}

fn read_limited_artifact(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    let size = file
        .metadata()
        .map_err(|error| TinyOneError::compile(format!("Artifact metadata error: {error}")))?
        .len();
    let max_bytes = MAX_ARTIFACT_BYTES.max(MAX_BINARY_ARTIFACT_BYTES);
    if size > max_bytes as u64 {
        return Err(TinyOneError::compile(format!(
            "Artifact rejected: byte size limit {max_bytes} exceeded (got {size})"
        )));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    if bytes.len() > max_bytes {
        return Err(TinyOneError::compile(format!("Artifact rejected: byte size limit {max_bytes} exceeded")));
    }
    Ok(bytes)
}
