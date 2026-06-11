pub(crate) mod artifact;
pub(crate) mod instr;
pub(crate) mod opcode;
pub(crate) mod peephole;
pub(crate) mod program;
pub(crate) mod verifier;

pub use instr::Instr;
pub use opcode::Op;
pub use peephole::PeepholeOptimizer;
pub use program::{Function, ModuleDef, ModuleImportDef, Program, StructDef, VerifiedProgram};
pub use verifier::BytecodeVerifier;
