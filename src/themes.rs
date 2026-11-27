//! Thèmes visuels et moteurs de recherche de CyberX.

use serde_json::{json, Map, Value};

const KEYS: [&str; 8] = [
    "--bg", "--surface", "--surface2", "--text", "--muted", "--accent", "--accent2", "--border",
];

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    /// bg, surface, surface2, text, muted, accent, accent2, border
    pub colors: [&'static str; 8],
}

pub const THEMES: &[Theme] = &[
    Theme { id: "cyber", name: "Cyber Néon", colors: ["#0a0e17", "#111827", "#1a2234", "#e6f1ff", "#7d8ba6", "#00e5ff", "#ff2bd6", "#22304a"] },
    Theme { id: "midnight", name: "Minuit", colors: ["#121214", "#1c1c20", "#26262c", "#ececf1", "#9a9aa6", "#fb542b", "#8b5cf6", "#2e2e35"] },
    Theme { id: "light", name: "Clair", colors: ["#eef0f4", "#ffffff", "#e3e6ed", "#1a1d24", "#6b7280", "#2563eb", "#db2777", "#d5d9e2"] },
    Theme { id: "matrix", name: "Matrix", colors: ["#000a03", "#03140a", "#072315", "#b7ffc8", "#4f8f60", "#00ff66", "#a3ff00", "#0d3a1d"] },
    Theme { id: "sunset", name: "Coucher de soleil", colors: ["#1b1020", "#261530", "#34203f", "#ffe9e0", "#b892a8", "#ff7a45", "#ff3d81", "#43284f"] },
    Theme { id: "ocean", name: "Océan", colors: ["#06141d", "#0b2130", "#112e42", "#e0f4ff", "#7fa3b8", "#38bdf8", "#34d399", "#1a3a50"] },
];

pub fn get(id: &str) -> &'static Theme {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}

/// Variables CSS du thème, pour l'interface.
pub fn vars(id: &str) -> Value {
    let t = get(id);
    let mut m = Map::new();
    for (k, v) in KEYS.iter().zip(t.colors.iter()) {
        m.insert((*k).to_string(), json!(v));
    }
    Value::Object(m)
}

/// Bloc CSS `:root{...}` pour la page « Nouvel onglet ».
pub fn css(id: &str) -> String {
    let t = get(id);
    let body: String = KEYS
        .iter()
        .zip(t.colors.iter())
        .map(|(k, v)| format!("{k}:{v};"))
        .collect();
    format!(":root{{{body}}}")
}

pub fn list() -> Value {
    Value::Array(
        THEMES
            .iter()
            .map(|t| json!({ "id": t.id, "name": t.name, "colors": t.colors }))
            .collect(),
    )
}

pub struct Engine {
    pub id: &'static str,
    pub name: &'static str,
    pub prefix: &'static str,
}

pub const ENGINES: &[Engine] = &[
    Engine { id: "duckduckgo", name: "DuckDuckGo", prefix: "https://duckduckgo.com/?q=" },
    Engine { id: "brave", name: "Brave Search", prefix: "https://search.brave.com/search?q=" },
    Engine { id: "startpage", name: "Startpage", prefix: "https://www.startpage.com/do/search?q=" },
    Engine { id: "qwant", name: "Qwant", prefix: "https://www.qwant.com/?q=" },
    Engine { id: "google", name: "Google", prefix: "https://www.google.com/search?q=" },
];

pub fn engine(id: &str) -> &'static Engine {
    ENGINES.iter().find(|e| e.id == id).unwrap_or(&ENGINES[0])
}

pub fn engines_list() -> Value {
    Value::Array(
        ENGINES
            .iter()
            .map(|e| json!({ "id": e.id, "name": e.name }))
            .collect(),
    )
}
