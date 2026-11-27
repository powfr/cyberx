//! Bouclier CyberX : bloqueur de pubs et de traqueurs basé sur `adblock-rust`
//! (le moteur open source de Brave) et les listes EasyList, EasyPrivacy et Liste FR.

use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime},
};
use tao::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Listes de filtres téléchargées puis gardées en cache.
const LISTS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
    ("liste_fr.txt", "https://easylist-downloads.adblockplus.org/liste_fr.txt"),
];

/// Les listes sont re-téléchargées si elles ont plus de 4 jours.
const MAX_AGE: Duration = Duration::from_secs(4 * 24 * 3600);

pub struct AdState {
    pub engine: Option<adblock::Engine>,
    pub enabled: bool,
    pub allow: HashSet<String>,
    pub rules: usize,
}

impl AdState {
    pub fn new(enabled: bool, allow: &[String]) -> Self {
        Self { engine: None, enabled, allow: allow.iter().cloned().collect(), rules: 0 }
    }

    /// Le bouclier est-il actif sur la page `page_url` ?
    pub fn active_on(&self, page_url: &str) -> bool {
        self.enabled && self.engine.is_some() && !self.allow.contains(&host(page_url))
    }

    pub fn should_block(&self, url: &str, page_url: &str, kind: &str) -> bool {
        if !url.starts_with("http") || !self.active_on(page_url) {
            return false;
        }
        let Some(engine) = &self.engine else { return false };
        match adblock::request::Request::new(url, page_url, kind) {
            Ok(req) => engine.check_network_request(&req).matched,
            Err(_) => false,
        }
    }

    /// Script qui masque les emplacements de pub restants sur la page.
    pub fn cosmetic_script(&self, page_url: &str) -> Option<String> {
        if !self.active_on(page_url) {
            return None;
        }
        let engine = self.engine.as_ref()?;
        let res = engine.url_cosmetic_resources(page_url);
        if res.hide_selectors.is_empty() {
            return None;
        }
        // Par paquets : un sélecteur invalide n'annule que son paquet.
        let selectors: Vec<&String> = res.hide_selectors.iter().collect();
        let css: String = selectors
            .chunks(40)
            .map(|c| {
                let list: Vec<&str> = c.iter().map(|s| s.as_str()).collect();
                format!("{}{{display:none!important}}\n", list.join(","))
            })
            .collect();
        let css_js = serde_json::to_string(&css).ok()?;
        Some(format!(
            "(function(){{var s=document.createElement('style');s.id='cyberx-shield';\
             s.textContent={css_js};(document.head||document.documentElement).appendChild(s);}})();"
        ))
    }

    pub fn build(&mut self, rules: &[String]) {
        let mut set = adblock::lists::FilterSet::new(false);
        set.add_filters(rules, adblock::lists::ParseOptions::default());
        self.engine = Some(adblock::Engine::from_filter_set(set, true));
        self.rules = rules.len();
    }
}

/// Nom de domaine d'une adresse, sans « www. ».
pub fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or("");
    let h = rest.split(['/', '?', '#']).next().unwrap_or("");
    let h = h.rsplit('@').next().unwrap_or(h);
    let h = h.split(':').next().unwrap_or(h);
    h.trim_start_matches("www.").to_lowercase()
}

/// Charge les listes (cache ou téléchargement) dans un fil séparé,
/// puis envoie les règles à la boucle principale.
pub fn spawn_load(dir: PathBuf, force: bool, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let _ = fs::create_dir_all(&dir);
        let mut rules: Vec<String> = Vec::new();
        for (name, url) in LISTS {
            let path = dir.join(name);
            let fresh = !force
                && fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|age| age < MAX_AGE)
                    .unwrap_or(false);

            let text = if fresh {
                fs::read_to_string(&path).ok()
            } else {
                match download(url) {
                    Some(t) => {
                        let _ = fs::write(&path, &t);
                        Some(t)
                    }
                    // Pas de réseau : on garde l'ancienne version si elle existe.
                    None => fs::read_to_string(&path).ok(),
                }
            };

            if let Some(t) = text {
                rules.extend(
                    t.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('['))
                        .map(str::to_string),
                );
            }
        }
        let _ = proxy.send_event(UserEvent::FiltersReady(rules));
    });
}

fn download(url: &str) -> Option<String> {
    ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .ok()?
        .into_string()
        .ok()
}
