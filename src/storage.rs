//! Sauvegarde locale : historique, favoris et paramètres (fichiers JSON).
//! Emplacement sous Windows : %APPDATA%\CyberX\

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_HISTORY: usize = 5000;

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub ts: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub ts: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub search: String,
    /// Bouclier (bloqueur de pubs) activé.
    pub adblock: bool,
    /// Sites sur lesquels le bouclier est désactivé.
    pub allowlist: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: "cyber".into(), search: "duckduckgo".into(), adblock: true, allowlist: Vec::new() }
    }
}

/// Onglets ouverts (hors navigation privée), rouverts au lancement.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<String>,
    pub active: usize,
}

pub struct Store {
    dir: PathBuf,
    pub history: Vec<HistoryEntry>,
    pub bookmarks: Vec<Bookmark>,
    pub settings: Settings,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Store {
    pub fn load() -> Self {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CyberX");
        let _ = fs::create_dir_all(&dir);
        Self {
            history: read_json(&dir, "history.json"),
            bookmarks: read_json(&dir, "bookmarks.json"),
            settings: read_json(&dir, "settings.json"),
            dir,
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) {
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        if let Ok(data) = serde_json::to_vec_pretty(value) {
            if fs::write(&tmp, data).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }

    // ---------- Historique ----------

    pub fn add_history(&mut self, url: &str, title: &str) {
        let title = if title.is_empty() { url } else { title };
        match self.history.last_mut() {
            Some(last) if last.url == url => {
                last.ts = now();
                last.title = title.to_string();
            }
            _ => self.history.push(HistoryEntry { url: url.into(), title: title.into(), ts: now() }),
        }
        if self.history.len() > MAX_HISTORY {
            let extra = self.history.len() - MAX_HISTORY;
            self.history.drain(0..extra);
        }
        self.write("history.json", &self.history);
    }

    pub fn update_history_title(&mut self, url: &str, title: &str) {
        if title.is_empty() {
            return;
        }
        if let Some(last) = self.history.iter_mut().rev().take(5).find(|h| h.url == url) {
            if last.title != title {
                last.title = title.to_string();
                self.write("history.json", &self.history);
            }
        }
    }

    pub fn delete_history(&mut self, url: &str, ts: u64) {
        self.history.retain(|h| !(h.url == url && h.ts == ts));
        self.write("history.json", &self.history);
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.write("history.json", &self.history);
    }

    // ---------- Favoris ----------

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    pub fn toggle_bookmark(&mut self, url: &str, title: &str) {
        if self.is_bookmarked(url) {
            self.bookmarks.retain(|b| b.url != url);
        } else {
            let title = if title.is_empty() { url } else { title };
            self.bookmarks.push(Bookmark { url: url.into(), title: title.into(), ts: now() });
        }
        self.write("bookmarks.json", &self.bookmarks);
    }

    pub fn remove_bookmark(&mut self, url: &str) {
        self.bookmarks.retain(|b| b.url != url);
        self.write("bookmarks.json", &self.bookmarks);
    }

    // ---------- Paramètres ----------

    pub fn save_settings(&self) {
        self.write("settings.json", &self.settings);
    }

    // ---------- Session ----------

    pub fn load_session(&self) -> Session {
        read_json(&self.dir, "session.json")
    }

    pub fn save_session(&self, session: &Session) {
        self.write("session.json", session);
    }
}

fn read_json<T: DeserializeOwned + Default>(dir: &PathBuf, name: &str) -> T {
    fs::read(dir.join(name))
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}
