#[derive(Debug, Clone)]
pub(crate) struct SourceMap {
    filename: String,
    source: String,
    line_starts: Vec<usize>,
}

impl SourceMap {
    pub(crate) fn new(source: impl Into<String>, filename: impl Into<String>) -> Self {
        let source = source.into();
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            filename: filename.into(),
            source,
            line_starts,
        }
    }

    pub(crate) fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.source.len());
        let mut low = 0usize;
        let mut high = self.line_starts.len();
        while low + 1 < high {
            let mid = (low + high) / 2;
            if self.line_starts[mid] <= pos {
                low = mid;
            } else {
                high = mid;
            }
        }
        (low + 1, pos - self.line_starts[low] + 1)
    }

    pub(crate) fn format(&self, message: impl AsRef<str>, pos: usize, end: usize) -> String {
        let pos = pos.min(self.source.len());
        let (line, column) = self.line_col(pos);
        let line_start = self.line_starts[line - 1];
        let next_line_start = if line < self.line_starts.len() {
            self.line_starts[line]
        } else {
            self.source.len()
        };
        let line_text = self.source[line_start..next_line_start].trim_end_matches('\n');
        let span_end = end.max(pos + 1).min(next_line_start);
        let width = span_end.saturating_sub(pos).max(1);
        let caret = format!(
            "{}{}",
            " ".repeat(column.saturating_sub(1)),
            "^".repeat(width)
        );
        format!(
            "{}:{}:{}: {}\n{}\n{}",
            self.filename,
            line,
            column,
            message.as_ref(),
            line_text,
            caret
        )
    }
}
