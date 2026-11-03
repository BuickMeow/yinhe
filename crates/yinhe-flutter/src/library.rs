//! 移动端音乐库索引：只读 `.yin` 头部的 project.json，按专辑分组。
//!
//! 不加载音符/音频，也不解压；文件来自最近打开列表（手机 SAF 无法扫描任意目录）。

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;

struct Entry {
    path: String,
    name: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    year: String,
    track_no: u16,
    disc_no: u16,
}

fn collect(paths: impl Iterator<Item = String>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        if !path.to_ascii_lowercase().ends_with(".yin") {
            continue;
        }
        let Ok(p) = yinhe_yin::read_project_file(&path) else {
            continue;
        };
        out.push(Entry {
            path,
            name: p.name,
            artist: p.artist,
            album: p.album,
            album_artist: p.album_artist,
            genre: p.genre,
            year: p.year,
            track_no: p.track_no,
            disc_no: p.disc_no,
        });
    }
    out
}

/// 返回 `{"groups":[{"album","album_artist","items":[...]}]}`。
pub fn library_json() -> String {
    let mut paths = crate::scene::recent_paths();
    // 当前打开的文件即使还没进最近列表，也放进库。
    if let Ok(s) = crate::scene::scene().lock()
        && let Some(doc) = s.document.as_ref()
        && let Some(p) = doc.file_path.as_ref()
        && Path::new(p).exists()
    {
        paths.insert(0, p.clone());
    }
    let entries = collect(paths.into_iter());
    let mut groups: BTreeMap<(String, String), Vec<Entry>> = BTreeMap::new();
    for e in entries {
        let artist = if e.album_artist.trim().is_empty() {
            e.artist.clone()
        } else {
            e.album_artist.clone()
        };
        groups.entry((e.album.clone(), artist)).or_default().push(e);
    }
    let mut out: Vec<serde_json::Value> = Vec::new();
    for ((album, album_artist), mut items) in groups {
        items.sort_by(|a, b| {
            let ta = if a.track_no == 0 {
                u16::MAX
            } else {
                a.track_no
            };
            let tb = if b.track_no == 0 {
                u16::MAX
            } else {
                b.track_no
            };
            ta.cmp(&tb)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let items: Vec<serde_json::Value> = items
            .into_iter()
            .map(|e| {
                json!({
                    "path": e.path,
                    "name": if e.name.is_empty() { "Untitled" } else { &e.name },
                    "artist": e.artist,
                    "album": e.album,
                    "album_artist": e.album_artist,
                    "genre": e.genre,
                    "year": e.year,
                    "track_no": e.track_no,
                    "disc_no": e.disc_no,
                })
            })
            .collect();
        out.push(json!({
            "album": album,
            "album_artist": album_artist,
            "items": items,
        }));
    }
    // 有专辑名的排前面，空专辑（单曲）放最后。
    out.sort_by(|a, b| {
        let aa = a["album"].as_str().unwrap_or("");
        let ba = b["album"].as_str().unwrap_or("");
        aa.is_empty()
            .cmp(&ba.is_empty())
            .then_with(|| aa.to_lowercase().cmp(&ba.to_lowercase()))
    });
    json!({ "groups": out }).to_string()
}
