//! Mise à jour automatique : CyberX vérifie la dernière version publiée sur GitHub
//! (github.com/powfr/cyberx/releases), la télécharge et remplace son propre .exe.

use std::{
    env, fs,
    io::Read,
    path::PathBuf,
    thread,
    time::Duration,
};
use tao::event_loop::EventLoopProxy;

use crate::UserEvent;

pub const REPO: &str = "powfr/cyberx";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSET: &str = "cyberx.exe";

fn parse(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

/// Supprime l'ancien exécutable laissé par la mise à jour précédente.
pub fn cleanup_old() {
    if let Ok(exe) = env::current_exe() {
        let _ = fs::remove_file(exe.with_file_name("cyberx_old.exe"));
        let _ = fs::remove_file(exe.with_file_name("cyberx_new.exe"));
    }
}

/// Vérifie en arrière-plan s'il existe une nouvelle version.
/// `manual` : vérification demandée depuis les paramètres (on affiche aussi « à jour »).
pub fn spawn_check(proxy: EventLoopProxy<UserEvent>, manual: bool) {
    thread::spawn(move || {
        if !manual {
            thread::sleep(Duration::from_secs(4));
        }
        let found = check();
        let _ = proxy.send_event(UserEvent::UpdateCheck { found, manual });
    });
}

/// Renvoie (version, lien de téléchargement) si une version plus récente existe.
fn check() -> Option<(String, String)> {
    let body = ureq::get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", "CyberX")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = v["tag_name"].as_str()?;
    if !is_newer(tag, VERSION) {
        return None;
    }
    let url = v["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str() == Some(ASSET))?["browser_download_url"]
        .as_str()?
        .to_string();
    Some((tag.trim_start_matches('v').to_string(), url))
}

/// Télécharge et installe la mise à jour en arrière-plan.
pub fn spawn_install(url: String, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let result = install(&url);
        let _ = proxy.send_event(UserEvent::UpdateResult(result));
    });
}

fn install(url: &str) -> Result<PathBuf, String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let response = ureq::get(url)
        .set("User-Agent", "CyberX")
        .timeout(Duration::from_secs(600))
        .call()
        .map_err(|e| format!("téléchargement impossible ({e})"))?;
    let mut data = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut data)
        .map_err(|e| format!("téléchargement interrompu ({e})"))?;

    // Vérification minimale : un vrai exécutable Windows commence par « MZ ».
    if data.len() < 500_000 || !data.starts_with(b"MZ") {
        return Err("le fichier téléchargé n'est pas valide".into());
    }

    // Windows autorise à renommer un .exe en cours d'exécution (mais pas à l'écraser).
    let new = exe.with_file_name("cyberx_new.exe");
    let old = exe.with_file_name("cyberx_old.exe");
    fs::write(&new, &data).map_err(|e| format!("écriture impossible ({e})"))?;
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("remplacement impossible ({e})"))?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("remplacement impossible ({e})"));
    }
    Ok(exe)
}
