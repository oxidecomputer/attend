use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, BufRead};
use std::num::NonZeroUsize;
use std::path::Path;
use std::str::FromStr;
use std::{fmt, fs};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// 1-based line number. Guaranteed ≥ 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Line(NonZeroUsize);

/// 1-based column (byte offset within line). Guaranteed ≥ 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Col(NonZeroUsize);

impl Line {
    /// Create from a raw value. Returns `None` for 0.
    pub fn new(n: usize) -> Option<Self> {
        NonZeroUsize::new(n).map(Self)
    }

    /// The underlying 1-based value.
    pub fn get(self) -> usize {
        self.0.get()
    }

    /// Subtract, clamping to 1 (not 0) so the invariant is preserved.
    pub fn saturating_sub(self, n: usize) -> Self {
        Self(NonZeroUsize::new(self.get().saturating_sub(n).max(1)).unwrap())
    }

    /// Add, clamping to `usize::MAX`. Result is always ≥ 1.
    pub fn saturating_add(self, n: usize) -> Self {
        Self(NonZeroUsize::new(self.get().saturating_add(n)).unwrap())
    }
}

impl Col {
    /// Create from a raw value. Returns `None` for 0.
    pub fn new(n: usize) -> Option<Self> {
        NonZeroUsize::new(n).map(Self)
    }

    /// The underlying 1-based value.
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for Col {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Line {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        let n: usize = s.parse().with_context(|| format!("bad line: {s:?}"))?;
        Self::new(n).with_context(|| format!("line must be >= 1: {s:?}"))
    }
}

impl FromStr for Col {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        let n: usize = s.parse().with_context(|| format!("bad col: {s:?}"))?;
        Self::new(n).with_context(|| format!("col must be >= 1: {s:?}"))
    }
}

/// A 1-based line:col position in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    /// 1-based line number.
    pub line: Line,
    /// 1-based column (byte offset within the line).
    pub col: Col,
}

impl Position {
    /// Construct from typed Line and Col.
    pub fn new(line: Line, col: Col) -> Self {
        Self { line, col }
    }

    /// Construct from raw usize values. Returns None if line or col is 0.
    pub fn of(line: usize, col: usize) -> Option<Self> {
        Some(Self::new(Line::new(line)?, Col::new(col)?))
    }
}

/// A selection range (or cursor when start == end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Start of the selection.
    pub start: Position,
    /// End of the selection (equal to start for a cursor).
    pub end: Position,
}

impl Selection {
    /// Whether this selection should be displayed as a cursor (just a position).
    /// Includes true zero-width cursors and single-character selections that
    /// editors sometimes report for cursors.
    pub fn is_cursor_like(&self) -> bool {
        self.start == self.end
            || (self.start.line == self.end.line && self.end.col.get() == self.start.col.get() + 1)
    }

    /// Line range spanned by this selection, normalized so first <= last.
    pub fn line_range(&self) -> (Line, Line) {
        if self.is_cursor_like() {
            (self.start.line, self.start.line)
        } else {
            (
                self.start.line.min(self.end.line),
                self.start.line.max(self.end.line),
            )
        }
    }

    /// Format for display in a group header: cursor-like selections show
    /// just the position, ranges show start-end.
    pub fn display_header(&self) -> String {
        if self.is_cursor_like() {
            self.start.to_string()
        } else {
            self.to_string()
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

impl fmt::Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

impl FromStr for Position {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        let (l, c) = s
            .split_once(':')
            .with_context(|| format!("expected line:col, got {s:?}"))?;
        Ok(Self::new(l.parse()?, c.parse()?))
    }
}

impl FromStr for Selection {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        if let Some(dash) = s.find('-') {
            let (left, right) = (&s[..dash], &s[dash + 1..]);
            if left.contains(':') && right.contains(':') {
                return Ok(Self {
                    start: left.parse()?,
                    end: right.parse()?,
                });
            }
        }
        let pos: Position = s.parse()?;
        Ok(Self {
            start: pos,
            end: pos,
        })
    }
}

impl Position {
    /// Convert sorted, deduplicated byte offsets to (line, col) positions in a
    /// single forward pass over the given reader. Handles `\n`, `\r\n`, and `\r`
    /// line endings. Offsets past EOF map to the final position.
    pub(crate) fn from_offsets(
        mut reader: impl BufRead,
        offsets: &[usize],
    ) -> anyhow::Result<Vec<Self>> {
        let max_offset = match offsets.last() {
            Some(&o) => o,
            None => return Ok(Vec::new()),
        };

        let mut result = Vec::with_capacity(offsets.len());
        let mut line = 1usize;
        let mut col = 1usize;
        let mut cursor = 0;
        let mut offset_idx = 0;
        let mut after_cr = false;

        while cursor <= max_offset && offset_idx < offsets.len() {
            // Emit positions for any offsets at the current cursor
            while offset_idx < offsets.len() && offsets[offset_idx] <= cursor {
                result.push(Position::of(line, col).unwrap());
                offset_idx += 1;
            }
            if offset_idx >= offsets.len() {
                break;
            }

            let buf = reader.fill_buf().context("read error")?;
            if buf.is_empty() {
                break;
            }
            let need = offsets[offset_idx] - cursor;
            let n = buf.len().min(need);
            for &b in &buf[..n] {
                match b {
                    b'\n' if after_cr => {
                        after_cr = false;
                    }
                    b'\n' => {
                        line += 1;
                        col = 1;
                    }
                    b'\r' => {
                        line += 1;
                        col = 1;
                        after_cr = true;
                    }
                    _ => {
                        col += 1;
                        after_cr = false;
                    }
                }
            }
            cursor += n;
            reader.consume(n);
        }

        // Emit remaining offsets (at or past EOF)
        while offset_idx < offsets.len() {
            result.push(Position::of(line, col).unwrap());
            offset_idx += 1;
        }

        Ok(result)
    }
}

impl Selection {
    /// Resolve raw byte-offset pairs to line:col selections from a reader.
    ///
    /// Deduplicates pairs, collects unique offsets for a single forward scan,
    /// then reconstructs selections from the offset-to-position lookup.
    pub(crate) fn resolve_from_reader(
        reader: impl BufRead,
        raw: &[(i64, i64)],
    ) -> anyhow::Result<Vec<Self>> {
        let mut seen: Vec<(i64, i64)> = raw.to_vec();
        seen.sort();
        seen.dedup();

        // Collect all unique offsets, sorted, for a single forward scan
        let mut all_offsets: Vec<usize> = seen
            .iter()
            .flat_map(|&(s, e)| [s as usize, e as usize])
            .collect();
        all_offsets.sort_unstable();
        all_offsets.dedup();

        let positions = Position::from_offsets(reader, &all_offsets)?;
        let lookup: HashMap<usize, Position> = all_offsets.into_iter().zip(positions).collect();

        seen.iter()
            .map(|&(s, e)| {
                let start = lookup
                    .get(&(s as usize))
                    .copied()
                    .with_context(|| format!("missing offset {s} in lookup"))?;
                let end = lookup
                    .get(&(e as usize))
                    .copied()
                    .with_context(|| format!("missing offset {e} in lookup"))?;
                Ok(Selection { start, end })
            })
            .collect()
    }

    /// Resolve raw byte-offset pairs to line:col selections by reading from a file path.
    pub(super) fn resolve(path: &Path, raw: &[(i64, i64)]) -> anyhow::Result<Vec<Self>> {
        let file =
            fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Self::resolve_from_reader(io::BufReader::new(file), raw)
    }
}

/// Make `path` relative to `cwd`, or return it unchanged if outside cwd.
pub(crate) fn relativize<'a>(path: &'a Path, cwd: Option<&Path>) -> Cow<'a, str> {
    let Some(cwd) = cwd else {
        return path.to_string_lossy();
    };
    match path.strip_prefix(cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => Cow::Borrowed("."),
        Ok(rel) => rel.to_string_lossy(),
        Err(_) => path.to_string_lossy(),
    }
}
