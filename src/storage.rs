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
    /// Espace secret : clé de chiffrement (sinon fichiers JSON ordinaires).
    key: Option<crate::crypto::SecretKey>,
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
        Self::open(dir, None)
    }

    /// Espace secret : mêmes données, chiffrées avec la clé tirée du code secret.
    pub fn load_secret(dir: PathBuf, key: crate::crypto::SecretKey) -> Self {
        Self::open(dir, Some(key))
    }

    fn open(dir: PathBuf, key: Option<crate::crypto::SecretKey>) -> Self {
        let _ = fs::create_dir_all(&dir);
        let k = key.as_ref();
        Self {
            history: read_json(&dir, k, "history.json"),
            bookmarks: read_json(&dir, k, "bookmarks.json"),
            settings: read_json(&dir, k, "settings.json"),
            key,
            dir,
        }
    }

    /// Change la clé (nouveau code secret) et réécrit toutes les données.
    pub fn rekey(&mut self, key: crate::crypto::SecretKey) {
        let session = self.load_session();
        self.key = Some(key);
        self.write("history.json", &self.history);
        self.write("bookmarks.json", &self.bookmarks);
        self.write("settings.json", &self.settings);
        self.save_session(&session);
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) {
        let name = file_name(self.key.as_ref(), name);
        let path = self.dir.join(&name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        if let Ok(data) = serde_json::to_vec_pretty(value) {
            let data = match &self.key {
                Some(k) => crate::crypto::seal(k, &data),
                None => data,
            };
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
        read_json(&self.dir, self.key.as_ref(), "session.json")
    }

    pub fn save_session(&self, session: &Session) {
        self.write("session.json", session);
    }
}

/// Dans l'espace secret, les fichiers portent des noms neutres.
fn file_name(key: Option<&crate::crypto::SecretKey>, name: &str) -> String {
    if key.is_none() {
        return name.to_string();
    }
    match name {
        "history.json" => "0.dat",
        "bookmarks.json" => "1.dat",
        "settings.json" => "2.dat",
        "session.json" => "3.dat",
        other => other,
    }
    .to_string()
}

fn read_json<T: DeserializeOwned + Default>(
    dir: &PathBuf,
    key: Option<&crate::crypto::SecretKey>,
    name: &str,
) -> T {
    fs::read(dir.join(file_name(key, name)))
        .ok()
        .and_then(|d| match key {
            Some(k) => crate::crypto::open(k, &d),
            None => Some(d),
        })
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}
