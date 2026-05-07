use crate::{Instr, JitOp};

pub(crate) const HOT_BACK_EDGE_THRESHOLD: u16 = 8;

#[derive(Debug, Clone)]
pub(crate) struct JitChunk {
    pub(crate) name: String,
    pub(crate) slot_count: usize,
    pub(crate) ops: Vec<JitOp>,
    pub(crate) edge_counts: Vec<u16>,
}

impl JitChunk {
    pub(crate) fn compile(name: impl Into<String>, slot_count: usize, code: &[Instr]) -> Self {
        let ops = code
            .iter()
            .copied()
            .map(JitOp::from_instr)
            .collect::<Vec<_>>();
        Self {
            name: name.into(),
            slot_count,
            edge_counts: vec![0; ops.len()],
            ops,
        }
    }

    pub(crate) fn promote_range(&mut self, start: usize, end: usize) -> usize {
        let start = start.min(self.ops.len());
        let end = end.min(self.ops.len());
        let mut changed = 0usize;
        for op in &mut self.ops[start..end] {
            let quickened = op.quickened();
            if quickened != *op {
                *op = quickened;
                changed += 1;
            }
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JitFunction {
    pub(crate) name: String,
    pub(crate) param_count: usize,
    pub(crate) slot_count: usize,
    pub(crate) chunk_index: usize,
}
