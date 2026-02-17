use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;

use anyhow::Context;

use crate::state::{Position, Selection};

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
        let mut line = 1;
        let mut col = 1;
        let mut cursor = 0;
        let mut offset_idx = 0;
        let mut after_cr = false;

        while cursor <= max_offset && offset_idx < offsets.len() {
            // Emit positions for any offsets at the current cursor
            while offset_idx < offsets.len() && offsets[offset_idx] <= cursor {
                result.push(Position { line, col });
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
            result.push(Position { line, col });
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
                    .cloned()
                    .context(format!("missing offset {s} in lookup"))?;
                let end = lookup
                    .get(&(e as usize))
                    .cloned()
                    .context(format!("missing offset {e} in lookup"))?;
                Ok(Selection { start, end })
            })
            .collect()
    }

    /// Resolve raw byte-offset pairs to line:col selections by reading from a file path.
    pub(super) fn resolve(path: &Path, raw: &[(i64, i64)]) -> anyhow::Result<Vec<Self>> {
        let file = fs::File::open(path).context(format!("cannot open {}", path.display()))?;
        Self::resolve_from_reader(io::BufReader::new(file), raw)
    }
}

/// Make `path` relative to `cwd`, or return it unchanged if outside cwd.
pub(super) fn relativize(path: &Path, cwd: Option<&Path>) -> String {
    let Some(cwd) = cwd else {
        return path.to_string_lossy().into_owned();
    };
    match path.strip_prefix(cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}
