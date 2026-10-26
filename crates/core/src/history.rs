//! Query history: every query run (from the app or the CLI), newest last, one JSON
//! object per line in `<config>/history.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Entries kept when the file is tidied up.
const KEEP: usize = 5_000;
/// Tidy up once the file holds this many.
const TIDY_AT: usize = 2 * KEEP;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// When it ran, in seconds since 1970 (UTC).
    pub at: u64,
    pub profile: String,
    pub sql: String,
    /// Statements that ran (including one that failed).
    pub statements: usize,
    pub elapsed_ms: u64,
    /// Rows returned (or affected) by the last statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct History {
    path: PathBuf,
    lock: Mutex<usize>,
}

impl History {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(0),
        }
    }

    pub fn add(&self, entry: &HistoryEntry) -> Result<()> {
        let mut appended = self.lock.lock().expect("history lock");
        let ctx = |e| CoreError::io(format!("writing {}", self.path.display()), e);
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(ctx)?;
        }
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&self.path).map_err(ctx)?;
        let mut line = serde_json::to_vec(entry).expect("history entry serialises");
        line.push(b'\n');
        f.write_all(&line).map_err(ctx)?;
        *appended += 1;
        // Now and then, check whether it's time to drop the oldest entries.
        if *appended % 200 == 1 {
            let all = self.read_all()?;
            if all.len() > TIDY_AT {
                self.rewrite(&all[all.len() - KEEP..])?;
            }
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<HistoryEntry>> {
        let f = match fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(CoreError::io(format!("reading {}", self.path.display()), e)),
        };
        // A damaged line (a crash mid-write, say) is skipped, not fatal.
        Ok(BufReader::new(f)
            .lines()
            .map_while(std::result::Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect())
    }

    fn rewrite(&self, entries: &[HistoryEntry]) -> Result<()> {
        let tmp = self.path.with_extension("jsonl.tmp");
        let ctx = |e| CoreError::io(format!("writing {}", self.path.display()), e);
        let mut out = Vec::new();
        for e in entries {
            out.extend(serde_json::to_vec(e).expect("history entry serialises"));
            out.push(b'\n');
        }
        fs::write(&tmp, out).map_err(ctx)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(ctx)?;
        }
        fs::rename(&tmp, &self.path).map_err(ctx)
    }

    /// Newest first, optionally only one profile's, and only those whose SQL
    /// contains `search` (ignoring case).
    pub fn list(
        &self,
        profile: Option<&str>,
        search: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        let _guard = self.lock.lock().expect("history lock");
        let search = search.map(str::to_lowercase).filter(|s| !s.is_empty());
        Ok(self
            .read_all()?
            .into_iter()
            .rev()
            .filter(|e| profile.is_none_or(|p| e.profile == p))
            .filter(|e| {
                search
                    .as_deref()
                    .is_none_or(|s| e.sql.to_lowercase().contains(s))
            })
            .take(limit)
            .collect())
    }

    /// Remove everything, or one profile's entries. Returns how many went.
    pub fn clear(&self, profile: Option<&str>) -> Result<usize> {
        let _guard = self.lock.lock().expect("history lock");
        let all = self.read_all()?;
        let keep: Vec<_> = all
            .iter()
            .filter(|e| profile.is_some_and(|p| e.profile != p))
            .cloned()
            .collect();
        self.rewrite(&keep)?;
        Ok(all.len() - keep.len())
    }
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(profile: &str, sql: &str, at: u64) -> HistoryEntry {
        HistoryEntry {
            at,
            profile: profile.into(),
            sql: sql.into(),
            statements: 1,
            elapsed_ms: 3,
            rows: Some(1),
            error: None,
        }
    }

    #[test]
    fn add_list_clear() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().join("history.jsonl"));
        assert!(h.list(None, None, 10).unwrap().is_empty());
        h.add(&entry("shop", "select * from orders", 1)).unwrap();
        h.add(&entry("crm", "select * from people", 2)).unwrap();
        h.add(&entry("shop", "update orders set x = 1", 3)).unwrap();
        let sql = |v: Vec<HistoryEntry>| v.into_iter().map(|e| e.sql).collect::<Vec<_>>();
        assert_eq!(
            sql(h.list(None, None, 10).unwrap()),
            [
                "update orders set x = 1",
                "select * from people",
                "select * from orders"
            ]
        );
        assert_eq!(
            sql(h.list(Some("shop"), Some("SELECT"), 10).unwrap()),
            ["select * from orders"]
        );
        assert_eq!(h.list(None, None, 2).unwrap().len(), 2);
        assert_eq!(h.clear(Some("shop")).unwrap(), 2);
        assert_eq!(
            sql(h.list(None, None, 10).unwrap()),
            ["select * from people"]
        );
        assert_eq!(h.clear(None).unwrap(), 1);
    }

    #[test]
    fn relative_times() {
        assert_eq!(ago(100, 110), "just now");
        assert_eq!(ago(0, 300), "5 min ago");
        assert_eq!(ago(0, 7300), "2 h ago");
        assert_eq!(ago(0, 90_000), "yesterday");
        assert_eq!(ago(0, 900_000), "10 days ago");
        assert_eq!(ago(500, 100), "just now");
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let h = History::new(path.clone());
        h.add(&entry("a", "select 1", 1)).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{not json\n")
            .unwrap();
        h.add(&entry("a", "select 2", 2)).unwrap();
        assert_eq!(h.list(None, None, 10).unwrap().len(), 2);
    }

    #[test]
    fn old_entries_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().join("history.jsonl"));
        let all: Vec<_> = (0..TIDY_AT as u64 + 5)
            .map(|i| entry("a", "x", i))
            .collect();
        h.rewrite(&all).unwrap();
        // The next add tidies up.
        h.add(&entry("a", "newest", 99_999)).unwrap();
        let kept = h.list(None, None, usize::MAX).unwrap();
        assert_eq!(kept.len(), KEEP);
        assert_eq!(kept[0].sql, "newest");
    }
}
