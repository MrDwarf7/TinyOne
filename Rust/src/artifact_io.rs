use std::fs;
use std::path::Path;

use crate::{Program, Result, TinyOneError};

pub fn load_artifact(path: impl AsRef<Path>) -> Result<Program> {
    let text = fs::read_to_string(path)
        .map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    let data = serde_json::from_str(&text)
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    Program::from_artifact(data)
}

pub fn write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let text = serde_json::to_string_pretty(&program.to_artifact())
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::compile(format!("Artifact write error: {error}")))
}
