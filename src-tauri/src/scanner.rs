use rusqlite::Connection;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub ext: Option<String>,
    pub children: Vec<FileNode>,
}

/// scan_recursive + mark-sweep prune: 스캔 후, 이번 스캔에서 못 본(= 물리적으로
/// 사라진) root 하위 인덱스 행을 제거한다. stale 인덱스가 남기는 유령 중복본
/// (content_hash 중복 오판)을 막는다. FE 트리·에이전트 scan_directory 도구가 공유한다.
pub fn scan_and_prune(root: &Path, max_depth: usize, conn: &Connection) -> Option<FileNode> {
    // 스캔 시작 시각 — scan_recursive 는 본 파일마다 last_seen=now(>=시작)로 갱신한다.
    let scan_start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let node = scan_recursive(root, 0, max_depth, conn)?;
    let root_str = root.to_string_lossy();
    let _ = conn.execute(
        "DELETE FROM files WHERE current_path LIKE ?1 || '%' AND last_seen < ?2",
        rusqlite::params![root_str.as_ref(), scan_start],
    );
    Some(node)
}

/// Returns None to signal "skip this entry" (symlink, hidden, permission error).
/// Errors reading individual children are swallowed so one bad entry doesn't abort the scan.
pub fn scan_recursive(
    path: &Path,
    depth: usize,
    max_depth: usize,
    conn: &Connection,
) -> Option<FileNode> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // Skip hidden entries (dot-files / dot-dirs)
    if name.starts_with('.') {
        return None;
    }

    let path_str = path.to_string_lossy().to_string();

    if path.is_dir() {
        let mut children = Vec::new();
        if depth < max_depth {
            if let Ok(read_dir) = fs::read_dir(path) {
                let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
                // Directories first, then files; both alphabetical
                entries.sort_by(|a, b| {
                    let a_dir = a.path().is_dir();
                    let b_dir = b.path().is_dir();
                    match (a_dir, b_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.file_name().cmp(&b.file_name()),
                    }
                });
                for entry in &entries {
                    if let Some(child) =
                        scan_recursive(&entry.path(), depth + 1, max_depth, conn)
                    {
                        children.push(child);
                    }
                }
            }
        }
        Some(FileNode {
            name,
            path: path_str,
            is_dir: true,
            size: None,
            ext: None,
            children,
        })
    } else if path.is_file() {
        let metadata = fs::metadata(path).ok()?;
        let size = metadata.len();

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let created_at = metadata
            .created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        // BLAKE3: stream through file — avoids loading large files into memory
        let content_hash = {
            let mut file = fs::File::open(path).ok()?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher).ok()?;
            hasher.finalize().to_hex().to_string()
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Upsert: insert or refresh size/mtime/hash/timestamps on path conflict
        let _ = conn.execute(
            "INSERT INTO files
                 (content_hash, current_path, size, mtime, created_at,
                  mime, ext, read_level, last_seen, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'metadata', ?8, ?9)
             ON CONFLICT(current_path) DO UPDATE SET
                 content_hash = excluded.content_hash,
                 size         = excluded.size,
                 mtime        = excluded.mtime,
                 last_seen    = excluded.last_seen,
                 indexed_at   = excluded.indexed_at",
            rusqlite::params![
                content_hash,
                path_str,
                size as i64,
                mtime,
                created_at,
                mime,
                ext,
                now,
                now
            ],
        );

        Some(FileNode {
            name,
            path: path_str,
            is_dir: false,
            size: Some(size),
            ext,
            children: vec![],
        })
    } else {
        None // symlinks, device files — skip
    }
}
