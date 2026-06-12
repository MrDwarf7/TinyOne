use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::bytecode::artifact::MAX_ARTIFACT_BYTES;
use crate::{Program, Result, TinyOneError};

pub fn load_artifact(path: impl AsRef<Path>) -> Result<Program> {
    let text = read_limited_artifact(path.as_ref())?;
    let data =
        serde_json::from_str(&text).map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    Program::from_artifact(data)
}

pub fn write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let text = serde_json::to_string_pretty(&program.to_artifact())
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::compile(format!("Artifact write error: {error}")))
}

fn read_limited_artifact(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    let size = file
        .metadata()
        .map_err(|error| TinyOneError::compile(format!("Artifact metadata error: {error}")))?
        .len();
    if size > MAX_ARTIFACT_BYTES as u64 {
        return Err(TinyOneError::compile(format!(
            "Artifact rejected: byte size limit {MAX_ARTIFACT_BYTES} exceeded (got {size})"
        )));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_ARTIFACT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(TinyOneError::compile(format!("Artifact rejected: byte size limit {MAX_ARTIFACT_BYTES} exceeded")));
    }
    String::from_utf8(bytes).map_err(|error| TinyOneError::compile(format!("Artifact must be UTF-8: {error}")))
}
