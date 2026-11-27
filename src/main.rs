// En mode release, pas de console noire derrière la fenêtre sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! CyberX — navigateur web en Rust.
//! Interface (onglets, barre d'adresse, panneaux) : une webview HTML dédiée.
//! Chaque onglet : une webview enfant (moteur WebView2 sous Windows).

mod shield;
#[cfg(windows)]
mod shield_win;
mod crypto;
mod passwords;
mod secret;
mod storage;
mod themes;
mod updater;

use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};
use shield::AdState;
use storage::{Session, Store};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, ResizeDirection, Window, WindowBuilder},
};
use wry::{
    dpi::{LogicalPosition, LogicalSize as WebLogicalSize},
    http::Request,
    PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder,
};

/// Hauteur de la barre du haut : onglets (38) + barre d'outils (46) + barre de favoris (30).
const TOP: f64 = 114.0;

/// Marge de redimensionnement autour de la fenêtre (sans bordure Windows).
const BORDER: f64 = 4.0;
/// Disposition « barre latérale » : largeur de la barre et marge autour de la page.
const SIDE_W: f64 = 264.0;
const CARD_GAP: f64 = 8.0;
/// Page « Nouvel onglet », servie par le protocole interne `cyberx`.
const NEWTAB_URL: &str = "http://cyberx.newtab/";
const NEWTAB_PRIVATE_URL: &str = "http://cyberx.newtab/private";

const UI_HTML: &str = include_str!("ui.html");
const NEWTAB_HTML: &str = include_str!("newtab.html");
const SUGGEST_HTML: &str = include_str!("suggest.html");
const PALETTE_HTML: &str = include_str!("palette.html");
const LOGO_SVG: &str = include_str!("../assets/logo.svg");

/// Script injecté dans les pages web : raccourcis clavier quand une page a le focus.
const SHORTCUTS_JS: &str = r#"
(function () {
  if (window.top !== window) return;
  window.addEventListener('keydown', function (e) {
    var k = (e.key || '').toLowerCase(), c = e.ctrlKey || e.metaKey, s = null;
    if (c && e.shiftKey && k === 'n') s = 'new_private';
    else if (c && k === 't') s = 'new_tab';
    else if (c && k === 'w') s = 'close_tab';
    else if (c && k === 'l') s = 'focus_address';
    else if (c && k === 'd') s = 'bookmark';
    else if (c && k === 'h') s = 'history';
    else if (c && k === 'tab') s = e.shiftKey ? 'prev_tab' : 'next_tab';
    else if (c && k === 'k') s = 'palette';
    else if (k === 'f11') s = 'toggle_fullscreen';
    else if (k === 'escape' && document.fullscreenElement) { document.exitFullscreen(); s = 'exit_fullscreen'; }
    if (s && window.ipc) { e.preventDefault(); e.stopPropagation(); window.ipc.postMessage(s); }
  }, true);
})();
"#;

/// Anti-pub YouTube : les pubs vidéo viennent des mêmes serveurs que les vidéos,
/// donc on les masque et on les passe automatiquement quand le bouclier est actif.
const YOUTUBE_JS: &str = r#"
(function () {
  if (window.top !== window || !/(^|\.)youtube\.com$/.test(location.hostname)) return;
  var css = 'ytd-ad-slot-renderer,ytd-in-feed-ad-layout-renderer,ytd-promoted-sparkles-web-renderer,' +
    'ytd-display-ad-renderer,ytd-banner-promo-renderer,ytd-statement-banner-renderer,ytd-companion-slot-renderer,' +
    'ytd-action-companion-ad-renderer,ytd-player-legacy-desktop-watch-ads-renderer,#player-ads,#masthead-ad,' +
    '.ytp-ad-overlay-container,.ytp-ad-image-overlay,ytd-rich-item-renderer:has(ytd-ad-slot-renderer)' +
    '{display:none!important}';
  var style = null, mutedByUs = false;
  setInterval(function () {
    var on = window.__cyberxShield === true;
    if (on && !style && document.documentElement) {
      style = document.createElement('style'); style.textContent = css;
      document.documentElement.appendChild(style);
    }
    if (!on && style) { style.remove(); style = null; }
    var player = document.querySelector('.html5-video-player');
    var video = player && player.querySelector('video');
    var ad = !!(player && player.classList.contains('ad-showing'));
    if (on && ad && video) {
      var skip = document.querySelector('.ytp-skip-ad-button,.ytp-ad-skip-button,.ytp-ad-skip-button-modern');
      if (skip) skip.click();
      if (!video.muted) { video.muted = true; mutedByUs = true; }
      if (isFinite(video.duration) && video.duration > 0) video.currentTime = video.duration;
    } else if (!ad && mutedByUs && video) {
      video.muted = false; mutedByUs = false;
    }
  }, 250);
})();
"#;

enum UserEvent {
    UiIpc(String),
    Shortcut(u32, String),
    PageLoad { tab: u32, url: String, finished: bool },
    Title { tab: u32, title: String },
    NewWindow { url: String, private: bool },
    /// Une requête de pub ou de traqueur a été bloquée dans cet onglet.
    Blocked(u32),
    /// Les listes de filtres sont chargées.
    FiltersReady(Vec<String>),
    /// Une page (ex. YouTube) entre ou sort du plein écran.
    Fullscreen(u32, bool),
    /// Résultat de la recherche de mise à jour : (version, lien) si disponible.
    UpdateCheck { found: Option<(String, String)>, manual: bool },
    /// Clic sur une suggestion de la barre d'adresse : {"url":…} ou {"search":…}.
    SuggestPick(String),
    /// Recalculer la mise en page (quelques instants après un passage en plein écran).
    Relayout,
    /// Battement régulier : écritures regroupées et compteur du bouclier.
    Tick,
    /// Mise à jour installée (chemin de l'exécutable) ou message d'erreur.
    UpdateResult(Result<std::path::PathBuf, String>),
}

struct Tab {
    id: u32,
    webview: WebView,
    title: String,
    url: String,
    private: bool,
    loading: bool,
    blocked: u32,
}

struct Browser {
    // Les webviews sont déclarées avant la fenêtre pour être détruites avant elle.
    ui: WebView,
    tabs: Vec<Tab>,
    /// Espace secret ouvert : données des sites séparées (déclaré après `tabs` pour être libéré après eux).
    secret_ctx: Option<WebContext>,
    /// Code de l'espace secret ouvert (le retaper le referme).
    secret_code: Option<String>,
    /// Données normales mises de côté pendant que l'espace secret est ouvert.
    parked: Option<(Store, passwords::Vault)>,
    active: usize,
    next_id: u32,
    panel_open: bool,
    fullscreen: bool,
    was_maximized: bool,
    /// La fenêtre était agrandie avant le plein écran : on la ré-agrandit en sortant.
    max_before_fullscreen: bool,
    /// Session (onglets ouverts) à réécrire au prochain battement.
    session_dirty: bool,
    /// Compteur du bouclier à rafraîchir au prochain battement.
    blocked_dirty: bool,
    ticks: u32,
    should_exit: bool,
    /// Mise à jour disponible : (version, lien).
    update: Option<(String, String)>,
    /// Message affiché dans la barre : « Mise à jour en cours… », « à jour », erreur.
    update_msg: Option<String>,
    /// Liste de suggestions affichée sous la barre d'adresse (créée au premier usage).
    suggest: Option<WebView>,
    /// Palette de commandes (Ctrl+K), créée au premier usage.
    palette: Option<WebView>,
    palette_open: std::cell::Cell<bool>,
    /// Contenu de la page « Nouvel onglet » : [normal, privé].
    newtab: Rc<RefCell<[String; 2]>>,
    vault: passwords::Vault,
    store: Store,
    ad: Rc<RefCell<AdState>>,
    proxy: EventLoopProxy<UserEvent>,
    window: Window,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: WebLogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

/// Options du moteur, identiques pour toutes les vues (WebView2 l'exige).
static BROWSER_ARGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Applique les options du moteur à une vue en construction.
fn engine_options(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        if let Some(args) = BROWSER_ARGS.get() {
            return builder.with_additional_browser_args(args.clone());
        }
    }
    builder
}

fn is_internal(url: &str) -> bool {
    url.starts_with("http://cyberx.") || url.starts_with("https://cyberx.")
}

fn is_web(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && !is_internal(url)
}

/// Ce qu'on affiche dans la barre d'adresse (rien pour la page « Nouvel onglet »).
fn display_url(url: &str) -> String {
    if url.starts_with("data:") || url.starts_with("about:") || is_internal(url) {
        String::new()
    } else {
        url.to_string()
    }
}

/// JSON sûr à insérer dans une balise <script>.
fn js_json(v: &Value) -> String {
    v.to_string().replace('<', "\\u003c")
}

fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Browser {
    fn logical_size(&self) -> (f64, f64) {
        let s = self
            .window
            .inner_size()
            .to_logical::<f64>(self.window.scale_factor());
        (s.width, s.height)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    // ---------------- Onglets ----------------

    fn newtab_url(&self, private: bool) -> String {
        let engine = themes::engine(&self.store.settings.search);
        let tiles: Vec<Value> = self
            .store
            .bookmarks
            .iter()
            .take(8)
            .map(|b| json!({ "url": b.url, "title": b.title }))
            .collect();
        let html = NEWTAB_HTML
            .replace("{{LOGO}}", LOGO_SVG)
            .replace("{{THEME_CSS}}", &themes::css(&self.store.settings.theme))
            .replace("{{SEARCH_PREFIX}}", &js_json(&json!(engine.prefix)))
            .replace("{{ENGINE_NAME}}", &js_json(&json!(engine.name)))
            .replace("{{TILES}}", &js_json(&json!(tiles)))
            .replace("{{PRIVATE}}", if private { "true" } else { "false" });
        self.newtab.borrow_mut()[private as usize] = html;
        if private { NEWTAB_PRIVATE_URL } else { NEWTAB_URL }.to_string()
    }

    fn create_tab(&mut self, url: Option<String>, private: bool) {
        self.set_fullscreen(false);
        let id = self.next_id;
        self.next_id += 1;
        let (w, h) = self.logical_size();
        let start = url.unwrap_or_else(|| self.newtab_url(private));

        let p_ipc = self.proxy.clone();
        let p_load = self.proxy.clone();
        let p_title = self.proxy.clone();
        let newtab = self.newtab.clone();

        // Onglet de l'espace secret : cookies et connexions séparés de l'espace normal.
        let builder = match self.secret_ctx.as_mut() {
            Some(ctx) if !private => WebViewBuilder::with_web_context(ctx),
            _ => WebViewBuilder::new(),
        };
        let builder = engine_options(builder);
        let built = builder
            .with_url(start.clone())
            .with_bounds(rect(0.0, TOP, w, (h - TOP).max(1.0)))
            .with_visible(false)
            .with_incognito(private)
            .with_initialization_script(SHORTCUTS_JS)
            .with_initialization_script(YOUTUBE_JS)
            // Pages internes : http://cyberx.newtab/ (et /private).
            .with_custom_protocol("cyberx".into(), move |_id, req| {
                let private = req.uri().path().starts_with("/private");
                let body = newtab.borrow()[private as usize].clone().into_bytes();
                wry::http::Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Cache-Control", "no-store")
                    .body(std::borrow::Cow::Owned(body))
                    .unwrap()
            })
            .with_ipc_handler(move |req: Request<String>| {
                let _ = p_ipc.send_event(UserEvent::Shortcut(id, req.body().clone()));
            })
            .with_on_page_load_handler(move |ev, url| {
                let finished = matches!(ev, PageLoadEvent::Finished);
                let _ = p_load.send_event(UserEvent::PageLoad { tab: id, url, finished });
            })
            .with_document_title_changed_handler(move |title| {
                let _ = p_title.send_event(UserEvent::Title { tab: id, title });
            })
            .build_as_child(&self.window);

        match built {
            Ok(webview) => {
                #[cfg(windows)]
                shield_win::attach(&webview, id, private, self.ad.clone(), self.proxy.clone());
                self.tabs.push(Tab {
                    id,
                    webview,
                    title: if private { "Navigation privée".into() } else { "Nouvel onglet".into() },
                    url: start,
                    private,
                    loading: true,
                    blocked: 0,
                });
                self.active = self.tabs.len() - 1;
                self.layout();
            }
            Err(e) => eprintln!("Impossible de créer l'onglet : {e}"),
        }
    }

    fn close_tab(&mut self, id: u32) {
        self.set_fullscreen(false);
        let Some(idx) = self.tabs.iter().position(|t| t.id == id) else { return };
        drop(self.tabs.remove(idx));
        if self.tabs.is_empty() {
            self.should_exit = true;
            return;
        }
        if idx < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.layout();
    }

    fn switch_to_id(&mut self, id: u32) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            self.set_fullscreen(false);
            self.active = idx;
            self.panel_open = false;
            self.layout();
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n < 2 {
            return;
        }
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
        self.layout();
    }

    fn layout(&self) {
        if self.panel_open || self.fullscreen {
            self.hide_suggest();
            self.hide_palette();
        }
        let (w, h) = self.logical_size();
        if self.fullscreen {
            let (w, h) = self.monitor_size().unwrap_or((w, h));
            let _ = self.ui.set_visible(false);
            for (i, t) in self.tabs.iter().enumerate() {
                let _ = t.webview.set_bounds(rect(0.0, 0.0, w, h));
                let _ = t.webview.set_visible(i == self.active);
            }
            return;
        }
        // Fenêtre non maximisée : une fine marge autour sert à la redimensionner.
        let b = if self.window.is_maximized() { 0.0 } else { BORDER };
        let page = if self.side_layout() {
            // Barre latérale à gauche, page en « carte » détachée du bord.
            let ui_w = if self.panel_open { w - 2.0 * b } else { SIDE_W };
            let _ = self.ui.set_bounds(rect(b, b, ui_w, h - 2.0 * b));
            (b + SIDE_W, b + CARD_GAP, w - 2.0 * b - SIDE_W - CARD_GAP, h - 2.0 * b - 2.0 * CARD_GAP)
        } else {
            let ui_h = if self.panel_open { h - 2.0 * b } else { TOP - b };
            let _ = self.ui.set_bounds(rect(b, b, w - 2.0 * b, ui_h));
            (b, TOP, w - 2.0 * b, h - TOP - b)
        };
        let _ = self.ui.set_visible(true);
        for (i, t) in self.tabs.iter().enumerate() {
            let _ = t.webview.set_bounds(rect(page.0, page.1, page.2, page.3));
            let _ = t.webview.set_visible(i == self.active && !self.panel_open);
        }
    }

    fn side_layout(&self) -> bool {
        self.store.settings.layout == "side"
    }

    fn set_fullscreen(&mut self, on: bool) {
        if self.fullscreen == on {
            return;
        }
        self.fullscreen = on;
        if on {
            // Une fenêtre agrandie passe mal en plein écran sous Windows : on la restaure d'abord.
            self.max_before_fullscreen = self.window.is_maximized();
            if self.max_before_fullscreen {
                self.window.set_maximized(false);
            }
        }
        #[cfg(windows)]
        shield_win::style_window_frame(&self.window, on);
        self.window
            .set_fullscreen(if on { Some(Fullscreen::Borderless(None)) } else { None });
        if !on {
            // La page est sortie du plein écran via la fenêtre : on la prévient aussi.
            self.eval_active("if (document.fullscreenElement) document.exitFullscreen()");
            if self.max_before_fullscreen {
                self.window.set_maximized(true);
            }
        }
        self.layout();
        if on {
            if let Some(t) = self.active_tab() {
                let _ = t.webview.focus();
            }
        }
        // La fenêtre change de taille un peu après : on recale la page plusieurs fois.
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            for ms in [60, 250, 700] {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                let _ = proxy.send_event(UserEvent::Relayout);
            }
        });
    }

    /// Taille de l'écran (en plein écran, plus fiable que la taille de la fenêtre en cours de changement).
    fn monitor_size(&self) -> Option<(f64, f64)> {
        let m = self.window.current_monitor()?;
        let s = m.size().to_logical::<f64>(m.scale_factor());
        Some((s.width, s.height))
    }

    fn apply_window_background(&self) {
        let hex = themes::get(&self.store.settings.theme).colors[0].trim_start_matches('#');
        let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
        self.window.set_background_color(Some((c(0), c(2), c(4), 255)));
    }

    /// Indique au script anti-pub YouTube si le bouclier est actif sur chaque onglet.
    fn refresh_shield_flags(&self) {
        let ad = self.ad.borrow();
        for t in &self.tabs {
            let on = ad.active_on(&t.url);
            let _ = t.webview.evaluate_script(&format!("window.__cyberxShield = {on};"));
        }
    }

    // ---------------- Navigation ----------------

    fn resolve_input(&self, input: &str, private: bool) -> String {
        let t = input.trim();
        if t.is_empty() {
            return self.newtab_url(private);
        }
        let lower = t.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file://") {
            return t.to_string();
        }
        let looks_like_host = !t.contains(' ')
            && (lower.starts_with("localhost") || (t.contains('.') && !t.starts_with('.') && !t.ends_with('.')));
        if looks_like_host {
            return format!("https://{t}");
        }
        let engine = themes::engine(&self.store.settings.search);
        format!("{}{}", engine.prefix, encode_query(t))
    }

    fn load_in_active(&mut self, url: &str) {
        self.panel_open = false;
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.loading = true;
            let _ = t.webview.load_url(url);
        }
        self.layout();
    }

    fn eval_active(&self, js: &str) {
        if let Some(t) = self.active_tab() {
            let _ = t.webview.evaluate_script(js);
        }
    }

    fn toggle_bookmark(&mut self) {
        let Some(t) = self.tabs.get(self.active) else { return };
        if is_web(&t.url) {
            let (url, title) = (t.url.clone(), t.title.clone());
            self.store.toggle_bookmark(&url, &title);
        }
    }

    fn open_panel(&mut self, section: &str) {
        self.panel_open = true;
        self.layout();
        let _ = self
            .ui
            .evaluate_script(&format!("window.cyberx && cyberx.openSection({})", js_json(&json!(section))));
    }

    fn focus_address(&self) {
        let _ = self.ui.focus();
        let _ = self.ui.evaluate_script("window.cyberx && cyberx.focusAddress()");
    }

    // ---------------- Session ----------------

    /// Enregistre les onglets normaux (jamais les onglets privés).
    fn save_session(&self) {
        let mut session = Session::default();
        for (i, t) in self.tabs.iter().enumerate() {
            if t.private || !is_web(&t.url) {
                continue;
            }
            if i == self.active {
                session.active = session.tabs.len();
            }
            session.tabs.push(t.url.clone());
        }
        self.store.save_session(&session);
    }

    fn restore_session(&mut self) {
        let session = self.store.load_session();
        if session.tabs.is_empty() {
            self.create_tab(None, false);
            return;
        }
        for url in session.tabs {
            self.create_tab(Some(url), false);
        }
        self.active = session.active.min(self.tabs.len().saturating_sub(1));
        self.layout();
    }

    // ---------------- Bouclier ----------------

    fn toggle_site_shield(&mut self) {
        let Some(url) = self.active_tab().map(|t| t.url.clone()) else { return };
        if !is_web(&url) {
            return;
        }
        let host = shield::host(&url);
        {
            let list = &mut self.store.settings.allowlist;
            if let Some(pos) = list.iter().position(|h| *h == host) {
                list.remove(pos);
            } else {
                list.push(host);
            }
        }
        self.store.save_settings();
        let mut ad = self.ad.borrow_mut();
        ad.allow = self.store.settings.allowlist.iter().cloned().collect();
        drop(ad);
        self.refresh_shield_flags();
    }

    // ---------------- Espace secret ----------------

    /// Si `input` est le code secret : ouvre ou ferme l'espace secret. Rien n'est enregistré.
    fn try_secret(&mut self, input: &str) -> bool {
        let code = input.trim();
        if let Some(current) = self.secret_code.clone() {
            if code == current {
                self.exit_secret();
                return true;
            }
            return false;
        }
        let base = self.store.dir();
        match secret::unlock(&base, code) {
            Some(key) => {
                self.enter_secret(code.to_string(), key, secret::dir(&base));
                true
            }
            None => false,
        }
    }

    fn enter_secret(&mut self, code: String, key: crypto::SecretKey, dir: std::path::PathBuf) {
        self.save_session();
        self.store.flush();
        self.set_fullscreen(false);
        self.panel_open = false;
        self.hide_suggest();
        self.tabs.clear();
        let store = std::mem::replace(&mut self.store, Store::load_secret(dir.clone(), key));
        let vault = std::mem::replace(&mut self.vault, passwords::Vault::load_secret(&dir, key));
        self.parked = Some((store, vault));
        self.secret_ctx = Some(WebContext::new(Some(dir.join("web"))));
        self.secret_code = Some(code);
        self.after_profile_switch();
    }

    fn exit_secret(&mut self) {
        self.save_session();
        self.store.flush();
        self.set_fullscreen(false);
        self.panel_open = false;
        self.hide_suggest();
        self.tabs.clear();
        if let Some((store, vault)) = self.parked.take() {
            self.store = store;
            self.vault = vault;
        }
        self.secret_ctx = None;
        self.secret_code = None;
        self.update_msg = None;
        self.after_profile_switch();
    }

    fn after_profile_switch(&mut self) {
        {
            let mut ad = self.ad.borrow_mut();
            ad.enabled = self.store.settings.adblock;
            ad.allow = self.store.settings.allowlist.iter().cloned().collect();
        }
        self.apply_window_background();
        self.restore_session();
    }

    fn change_secret_code(&mut self, code: &str) {
        if self.secret_code.is_none() {
            return;
        }
        if !secret::valid_code(code) {
            self.update_msg = Some("Code refusé : au moins 4 caractères, sans espace.".into());
            return;
        }
        let key = secret::set_code(&self.store.dir(), code);
        self.store.rekey(key);
        self.vault.rekey(key);
        self.secret_code = Some(code.to_string());
        self.update_msg = Some("✓ Nouveau code secret enregistré. Retiens-le bien : sans lui, cet espace est illisible.".into());
    }

    // ---------------- Palette de commandes (Ctrl+K) ----------------

    fn hide_palette(&self) {
        if let Some(p) = &self.palette {
            let _ = p.set_visible(false);
        }
        self.palette_open.set(false);
    }

    fn palette_visible(&self) -> bool {
        self.palette_open.get()
    }

    fn close_palette(&self) {
        if self.palette_visible() {
            self.hide_palette();
            if let Some(t) = self.active_tab() {
                let _ = t.webview.focus();
            }
        }
    }

    fn toggle_palette(&mut self) {
        if self.palette_visible() {
            self.close_palette();
            return;
        }
        if self.fullscreen {
            return;
        }
        self.panel_open = false;
        self.layout();
        self.hide_suggest();
        if self.palette.is_none() {
            let p = self.proxy.clone();
            self.palette = engine_options(WebViewBuilder::new())
                .with_html(PALETTE_HTML)
                .with_visible(false)
                .with_bounds(rect(0.0, 0.0, 10.0, 10.0))
                .with_ipc_handler(move |req: Request<String>| {
                    let _ = p.send_event(UserEvent::UiIpc(req.body().clone()));
                })
                .build_as_child(&self.window)
                .ok();
        }
        let Some(palette) = &self.palette else { return };
        let (w, h) = self.logical_size();
        let pw = (w - 80.0).min(680.0);
        let ph = (h - 140.0).min(470.0);
        let _ = palette.set_bounds(rect((w - pw) / 2.0, 70.0, pw, ph));
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|t| json!({ "id": t.id, "title": t.title, "url": display_url(&t.url), "private": t.private }))
            .collect();
        let history: Vec<&storage::HistoryEntry> = self.store.history.iter().rev().take(800).collect();
        let data = json!({
            "tabs": tabs,
            "bookmarks": self.store.bookmarks,
            "history": history,
            "themes": themes::list(),
            "theme": themes::vars(&self.store.settings.theme),
            "engine": themes::engine(&self.store.settings.search).name,
        });
        let _ = palette.evaluate_script(&format!("open_palette({})", js_json(&data)));
        let _ = palette.set_visible(true);
        self.palette_open.set(true);
        #[cfg(windows)]
        shield_win::bring_to_front(palette);
        let _ = palette.focus();
    }

    // ---------------- Suggestions de la barre d'adresse ----------------

    fn hide_suggest(&self) {
        if let Some(s) = &self.suggest {
            let _ = s.set_visible(false);
        }
    }

    fn show_suggest(&mut self, v: &Value) {
        let items = v["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() || self.panel_open || self.fullscreen {
            self.hide_suggest();
            return;
        }
        if self.suggest.is_none() {
            let p = self.proxy.clone();
            self.suggest = engine_options(WebViewBuilder::new())
                .with_html(SUGGEST_HTML)
                .with_visible(false)
                .with_bounds(rect(0.0, 0.0, 10.0, 10.0))
                .with_ipc_handler(move |req: Request<String>| {
                    let _ = p.send_event(UserEvent::SuggestPick(req.body().clone()));
                })
                .build_as_child(&self.window)
                .ok();
        }
        let Some(s) = &self.suggest else { return };
        // Position reçue de l'interface, dans ses coordonnées : on ajoute son décalage dans la fenêtre.
        let b = if self.window.is_maximized() { 0.0 } else { BORDER };
        let x = v["x"].as_f64().unwrap_or(0.0) + b;
        let y = v["y"].as_f64().unwrap_or(80.0) + b;
        let w = v["w"].as_f64().unwrap_or(600.0);
        let h = items.len() as f64 * 36.0 + 10.0;
        let _ = s.set_bounds(rect(x, y, w, h));
        let engine = themes::engine(&self.store.settings.search).name;
        let _ = s.evaluate_script(&format!(
            "render({}, {}, {}, {})",
            js_json(&Value::Array(items)),
            v["sel"].as_i64().unwrap_or(-1),
            themes::vars(&self.store.settings.theme),
            js_json(&json!(engine)),
        ));
        let _ = s.set_visible(true);
        #[cfg(windows)]
        shield_win::bring_to_front(s);
    }

    /// Données des suggestions, envoyées seulement quand la barre d'adresse est utilisée.
    fn send_suggest_data(&self) {
        // Suggestions pour la barre d'adresse : favoris, puis pages récentes (sans doublon).
        let mut seen = std::collections::HashSet::new();
        let mut suggest: Vec<Value> = Vec::new();
        for b in &self.store.bookmarks {
            if seen.insert(b.url.as_str()) {
                suggest.push(json!({ "url": b.url, "title": b.title, "kind": "bm" }));
            }
        }
        for h in self.store.history.iter().rev() {
            if suggest.len() >= 400 {
                break;
            }
            if seen.insert(h.url.as_str()) {
                suggest.push(json!({ "url": h.url, "title": h.title, "kind": "hist" }));
            }
        }

        let _ = self
            .ui
            .evaluate_script(&format!("window.cyberx && cyberx.setSuggest({})", js_json(&Value::Array(suggest))));
    }

    fn pick_suggestion(&mut self, msg: &str) {
        self.hide_suggest();
        let Ok(v) = serde_json::from_str::<Value>(msg) else { return };
        let private = self.active_tab().map(|t| t.private).unwrap_or(false);
        let url = if let Some(u) = v["url"].as_str().filter(|u| is_web(u)) {
            u.to_string()
        } else if let Some(q) = v["search"].as_str() {
            self.resolve_input(q, private)
        } else {
            return;
        };
        self.load_in_active(&url);
        let _ = self.ui.evaluate_script("window.cyberx && cyberx.suggestDone()");
    }

    // ---------------- Messages ----------------

    fn handle(&mut self, ev: UserEvent) {
        match ev {
            // Événement très fréquent : mise à jour légère du compteur, sans tout redessiner.
            UserEvent::Blocked(tab) => {
                let active_id = self.active_tab().map(|t| t.id);
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.blocked += 1;
                    if Some(tab) == active_id {
                        self.blocked_dirty = true;
                    }
                }
                return;
            }
            UserEvent::FiltersReady(rules) => {
                self.ad.borrow_mut().build(&rules);
                self.refresh_shield_flags();
            }
            UserEvent::UpdateCheck { found, manual } => {
                if found.is_none() && manual {
                    self.update_msg = Some(format!("CyberX est à jour (version {}).", updater::VERSION));
                }
                if found.is_some() {
                    self.update_msg = None;
                }
                self.update = found;
                self.push_state();
                return;
            }
            UserEvent::UpdateResult(result) => {
                match result {
                    Ok(exe) => {
                        // Relancer la nouvelle version en gardant les onglets.
                        self.save_session();
                        self.store.flush();
                        if std::process::Command::new(&exe).spawn().is_ok() {
                            self.should_exit = true;
                            return;
                        }
                        self.update_msg = Some("Mise à jour installée : redémarrez CyberX.".into());
                    }
                    Err(e) => self.update_msg = Some(format!("Échec de la mise à jour : {e}")),
                }
                self.push_state();
                return;
            }
            UserEvent::SuggestPick(msg) => self.pick_suggestion(&msg),
            UserEvent::Relayout => {
                self.layout();
                return;
            }
            UserEvent::Tick => {
                self.ticks = self.ticks.wrapping_add(1);
                if self.blocked_dirty {
                    self.blocked_dirty = false;
                    if let Some(t) = self.active_tab() {
                        let _ = self
                            .ui
                            .evaluate_script(&format!("window.cyberx && cyberx.setBlocked({})", t.blocked));
                    }
                }
                if self.ticks % 4 == 0 {
                    if self.session_dirty {
                        self.session_dirty = false;
                        self.save_session();
                    }
                    self.store.flush();
                }
                return;
            }
            UserEvent::Fullscreen(tab, on) => {
                if self.active_tab().map(|t| t.id) == Some(tab) {
                    self.set_fullscreen(on);
                }
                return;
            }
            UserEvent::UiIpc(msg) => self.handle_ui(&msg),
            UserEvent::Shortcut(tab, s) => self.handle_shortcut(tab, &s),
            UserEvent::PageLoad { tab, url, finished } => {
                let mut record = None;
                let cosmetic = if finished { self.ad.borrow().cosmetic_script(&url) } else { None };
                let autofill = if finished { self.vault.autofill_script(&url) } else { None };
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.url = url.clone();
                    t.loading = !finished;
                    if !finished {
                        t.blocked = 0;
                    }
                    if let Some(js) = &cosmetic {
                        let _ = t.webview.evaluate_script(js);
                    }
                    if let Some(js) = &autofill {
                        let _ = t.webview.evaluate_script(js);
                    }
                    if finished {
                        let on = self.ad.borrow().active_on(&url);
                        let _ = t.webview.evaluate_script(&format!("window.__cyberxShield = {on};"));
                    }
                    if finished && !t.private && is_web(&url) {
                        record = Some((url, t.title.clone()));
                    }
                }
                if let Some((u, title)) = record {
                    self.store.add_history(&u, &title);
                }
            }
            UserEvent::Title { tab, title } => {
                let mut update = None;
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    if !title.is_empty() {
                        t.title = title.clone();
                    }
                    if !t.private && is_web(&t.url) {
                        update = Some(t.url.clone());
                    }
                }
                if let Some(u) = update {
                    self.store.update_history_title(&u, &title);
                }
            }
            UserEvent::NewWindow { url, private } => self.create_tab(Some(url), private),
        }
        self.session_dirty = true;
        self.push_state();
    }

    /// Raccourcis envoyés depuis une page web (liste fermée, onglet actif uniquement).
    fn handle_shortcut(&mut self, tab: u32, s: &str) {
        if self.active_tab().map(|t| t.id) != Some(tab) {
            return;
        }
        match s {
            "new_tab" => self.create_tab(None, false),
            "new_private" => self.create_tab(None, true),
            "close_tab" => self.close_tab(tab),
            "focus_address" => self.focus_address(),
            "bookmark" => self.toggle_bookmark(),
            "history" => self.open_panel("history"),
            "next_tab" => self.cycle_tab(true),
            "prev_tab" => self.cycle_tab(false),
            "palette" => self.toggle_palette(),
            "toggle_fullscreen" => {
                let on = !self.fullscreen;
                self.set_fullscreen(on);
            }
            "exit_fullscreen" => self.set_fullscreen(false),
            // Recherche depuis la page « Nouvel onglet » (seulement depuis cette page).
            s if s.starts_with("nav:") => {
                let from_newtab = self.active_tab().map(|t| t.url.starts_with("http://cyberx.newtab")).unwrap_or(false);
                let value = &s[4..];
                if from_newtab && !self.try_secret(value) {
                    let private = self.active_tab().map(|t| t.private).unwrap_or(false);
                    let url = self.resolve_input(value, private);
                    self.load_in_active(&url);
                }
            }
            _ => {}
        }
    }

    fn handle_ui(&mut self, msg: &str) {
        let Ok(v) = serde_json::from_str::<Value>(msg) else { return };
        let str_arg = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let id_arg = || v["id"].as_u64().unwrap_or(u64::MAX) as u32;

        match v["cmd"].as_str().unwrap_or("") {
            "ready" => {}
            "navigate" => {
                let value = str_arg("value");
                if !self.try_secret(&value) {
                    let private = self.active_tab().map(|t| t.private).unwrap_or(false);
                    let url = self.resolve_input(&value, private);
                    self.load_in_active(&url);
                }
            }
            "exit_secret" => {
                if self.secret_code.is_some() {
                    self.exit_secret();
                }
            }
            "secret_change_code" => self.change_secret_code(&str_arg("code")),
            "open_url" => {
                let url = str_arg("url");
                if is_web(&url) {
                    self.load_in_active(&url);
                }
            }
            "open_url_new_tab" => {
                let url = str_arg("url");
                if is_web(&url) {
                    self.panel_open = false;
                    // Onglet actif encore sur la page « Nouvel onglet » : on l'utilise plutôt que d'en ouvrir un autre.
                    let on_newtab = self.active_tab().map(|t| is_internal(&t.url)).unwrap_or(false);
                    if on_newtab {
                        self.load_in_active(&url);
                    } else {
                        let private = self.active_tab().map(|t| t.private).unwrap_or(false);
                        self.create_tab(Some(url), private);
                    }
                }
            }
            "back" => self.eval_active("history.back()"),
            "forward" => self.eval_active("history.forward()"),
            "reload" => self.eval_active("location.reload()"),
            "stop" => self.eval_active("window.stop()"),
            "home" => {
                let private = self.active_tab().map(|t| t.private).unwrap_or(false);
                let url = self.newtab_url(private);
                self.load_in_active(&url);
            }
            "new_tab" => {
                self.panel_open = false;
                self.create_tab(None, v["private"].as_bool().unwrap_or(false));
            }
            "close_tab" => self.close_tab(id_arg()),
            "switch_tab" => self.switch_to_id(id_arg()),
            "toggle_bookmark" => self.toggle_bookmark(),
            "remove_bookmark" => self.store.remove_bookmark(&str_arg("url")),
            "panel" => {
                self.panel_open = v["open"].as_bool().unwrap_or(false);
                self.layout();
            }
            "delete_history" => {
                let ts = v["ts"].as_u64().unwrap_or(0);
                self.store.delete_history(&str_arg("url"), ts);
            }
            "clear_history" => self.store.clear_history(),
            "suggest" => self.show_suggest(&v),
            "need_suggest" => self.send_suggest_data(),
            "palette" => self.toggle_palette(),
            "toggle_fullscreen" => {
                let on = !self.fullscreen;
                self.set_fullscreen(on);
            }
            "exit_fullscreen" => self.set_fullscreen(false),
            "palette_close" => self.close_palette(),
            "open_section" => self.open_panel(&str_arg("section")),
            "set_video_compat" => {
                self.store.settings.video_compat = v["on"].as_bool().unwrap_or(true);
                self.store.save_settings();
                self.update_msg = Some("Mode compatibilité vidéo modifié : redémarre CyberX pour l'appliquer.".into());
            }
            "set_layout" => {
                let id = str_arg("id");
                if id == "side" || id == "classic" {
                    self.store.settings.layout = id;
                    self.store.save_settings();
                    self.layout();
                }
            }
            "import_passwords" => {
                self.update_msg = Some(match self.vault.import_csv(&str_arg("csv")) {
                    Ok(n) => format!(
                        "✓ {n} mot(s) de passe importé(s). Pense à supprimer le fichier CSV exporté : il n'est pas chiffré."
                    ),
                    Err(e) => format!("Import impossible : {e}"),
                });
            }
            "clear_passwords" => self.vault.clear(),
            "set_theme" => {
                self.store.settings.theme = themes::get(&str_arg("id")).id.to_string();
                self.store.save_settings();
                self.apply_window_background();
            }
            "toggle_site_shield" => self.toggle_site_shield(),
            "install_update" => {
                if let Some((_, url)) = self.update.clone() {
                    self.update_msg = Some("Téléchargement de la mise à jour…".into());
                    updater::spawn_install(url, self.proxy.clone());
                }
            }
            "check_update" => {
                self.update_msg = Some("Recherche de mises à jour…".into());
                updater::spawn_check(self.proxy.clone(), true);
            }
            "dismiss_update" => {
                self.update = None;
                self.update_msg = None;
            }
            // Fenêtre sans bordure : déplacement, redimensionnement et boutons ─ ▢ ✕
            "drag" => {
                let _ = self.window.drag_window();
            }
            "resize_top" => {
                if !self.window.is_maximized() {
                    let _ = self.window.drag_resize_window(ResizeDirection::North);
                }
            }
            "minimize" => self.window.set_minimized(true),
            "toggle_maximize" => {
                let max = self.window.is_maximized();
                self.window.set_maximized(!max);
            }
            "close_window" => {
                self.save_session();
                self.store.flush();
                self.should_exit = true;
            }
            "remove_allow" => {
                let host = str_arg("host");
                self.store.settings.allowlist.retain(|h| *h != host);
                self.store.save_settings();
                self.ad.borrow_mut().allow.remove(&host);
                self.refresh_shield_flags();
            }
            "set_adblock" => {
                let on = v["on"].as_bool().unwrap_or(true);
                self.store.settings.adblock = on;
                self.store.save_settings();
                self.ad.borrow_mut().enabled = on;
                self.refresh_shield_flags();
            }
            "update_filters" => {
                shield::spawn_load(self.store.dir().join("filters"), true, self.proxy.clone());
            }
            "set_search" => {
                self.store.settings.search = themes::engine(&str_arg("id")).id.to_string();
                self.store.save_settings();
            }
            _ => {}
        }
    }

    /// Envoie l'état complet à l'interface.
    fn push_state(&self) {
        let active = self.active_tab();
        let raw_url = active.map(|t| t.url.clone()).unwrap_or_default();

        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "title": t.title,
                    "url": display_url(&t.url),
                    "private": t.private,
                    "loading": t.loading,
                })
            })
            .collect();

        // L'historique complet n'est envoyé que quand le panneau est ouvert.
        let history: Vec<&storage::HistoryEntry> = if self.panel_open {
            self.store.history.iter().rev().take(1500).collect()
        } else {
            Vec::new()
        };

        let ad = self.ad.borrow();
        let site_host = if is_web(&raw_url) { shield::host(&raw_url) } else { String::new() };
        let state = json!({
            "blocked": active.map(|t| t.blocked).unwrap_or(0),
            "adblock": ad.enabled,
            "filtersReady": ad.engine.is_some(),
            "filterRules": ad.rules,
            "host": site_host,
            "siteAllowed": !site_host.is_empty() && ad.allow.contains(&site_host),
            "tabs": tabs,
            "active": active.map(|t| t.id),
            "url": display_url(&raw_url),
            "private": active.map(|t| t.private).unwrap_or(false),
            "loading": active.map(|t| t.loading).unwrap_or(false),
            "bookmarked": self.store.is_bookmarked(&raw_url),
            "bookmarks": self.store.bookmarks,
            "history": history,
            "settings": self.store.settings,
            "theme": themes::vars(&self.store.settings.theme),
            "themes": themes::list(),
            "engines": themes::engines_list(),
            "panelOpen": self.panel_open,
            "secret": self.secret_code.is_some(),
            "maximized": self.window.is_maximized(),
            "layout": self.store.settings.layout,
            "videoCompat": self.store.settings.video_compat,
            "passwordCount": self.vault.creds.len(),
            "version": updater::VERSION,
            "update": self.update.as_ref().map(|(v, _)| v.clone()),
            "updateMsg": self.update_msg,
        });
        let _ = self
            .ui
            .evaluate_script(&format!("window.cyberx && cyberx.update({})", state));

        let title = active.map(|t| t.title.as_str()).unwrap_or("");
        self.window.set_title(&if title.is_empty() {
            "CyberX".to_string()
        } else {
            format!("{title} — CyberX")
        });
    }
}

/// Icône de la fenêtre : reprise de l'icône intégrée à l'exécutable (voir build.rs).
#[cfg(windows)]
fn window_icon() -> Option<tao::window::Icon> {
    use tao::platform::windows::IconExtWindows;
    tao::window::Icon::from_resource(1, None).ok()
}

#[cfg(not(windows))]
fn window_icon() -> Option<tao::window::Icon> {
    None
}

fn main() {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("CyberX")
        .with_window_icon(window_icon())
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(640.0, 400.0))
        .build(&event_loop)
        .expect("Impossible de créer la fenêtre");

    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let p_ui = proxy.clone();
    // Options du moteur (avant toute vue) : celles de wry par défaut, plus le mode compatibilité vidéo.
    let mut disabled = String::from("msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if Store::load().settings.video_compat {
        // Décodage matériel des vidéos protégées désactivé : évite l'écran noir (avec le son) sur Netflix.
        disabled.push_str(",HardwareSecureDecryption,HardwareSecureDecryptionExperiment,msPlayReadyHardwareDrm");
    }
    let _ = BROWSER_ARGS.set(format!("--disable-features={disabled}"));

    let ui = engine_options(WebViewBuilder::new())
        .with_html(UI_HTML.replace("{{LOGO}}", LOGO_SVG))
        .with_bounds(rect(0.0, 0.0, size.width, TOP))
        .with_ipc_handler(move |req: Request<String>| {
            let _ = p_ui.send_event(UserEvent::UiIpc(req.body().clone()));
        })
        .build_as_child(&window)
        .expect("Impossible de créer l'interface (WebView2 est-il installé ?)");

    let mut browser = Browser {
        ui,
        tabs: Vec::new(),
        active: 0,
        next_id: 1,
        panel_open: false,
        fullscreen: false,
        was_maximized: false,
        max_before_fullscreen: false,
        session_dirty: false,
        blocked_dirty: false,
        ticks: 0,
        should_exit: false,
        update: None,
        update_msg: None,
        suggest: None,
        palette: None,
        palette_open: std::cell::Cell::new(false),
        newtab: Rc::new(RefCell::new([String::new(), String::new()])),
        vault: passwords::Vault::load(&Store::load().dir()),
        secret_ctx: None,
        secret_code: None,
        parked: None,
        ad: Rc::new(RefCell::new(AdState::new(true, &[]))),
        store: Store::load(),
        proxy,
        window,
    };
    browser.ad = Rc::new(RefCell::new(AdState::new(
        browser.store.settings.adblock,
        &browser.store.settings.allowlist,
    )));
    updater::cleanup_old();
    updater::spawn_check(browser.proxy.clone(), false);
    browser.apply_window_background();
    #[cfg(windows)]
    shield_win::style_window_frame(&browser.window, false);
    shield::spawn_load(browser.store.dir().join("filters"), false, browser.proxy.clone());
    browser.restore_session();
    // Battement régulier (toutes les 0,5 s) pour les tâches regroupées.
    let tick_proxy = browser.proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if tick_proxy.send_event(UserEvent::Tick).is_err() {
            break;
        }
    });
    browser.push_state();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                browser.save_session();
                browser.store.flush();
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. }
            | Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { .. }, .. } => {
                browser.layout();
                // Mettre à jour l'icône ▢ / ❐ quand la fenêtre est (dé)maximisée.
                let max = browser.window.is_maximized();
                if max != browser.was_maximized {
                    browser.was_maximized = max;
                    browser.push_state();
                }
            }
            Event::UserEvent(ev) => browser.handle(ev),
            _ => {}
        }
        if browser.should_exit {
            browser.store.flush();
            *control_flow = ControlFlow::Exit;
        }
    });
}
