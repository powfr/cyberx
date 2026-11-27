//! Mots de passe : import depuis Brave / Chrome / Edge / Firefox (fichier CSV exporté),
//! stockage chiffré avec la protection de Windows (DPAPI, liée à ta session Windows)
//! et remplissage automatique des formulaires de connexion.

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct Cred {
    pub url: String,
    pub username: String,
    pub password: String,
}

pub struct Vault {
    path: PathBuf,
    /// Espace secret : chiffré avec la clé du code secret (sinon protection Windows).
    key: Option<crate::crypto::SecretKey>,
    pub creds: Vec<Cred>,
}

impl Vault {
    pub fn load(dir: &PathBuf) -> Self {
        Self::open(dir.join("passwords.bin"), None)
    }

    pub fn load_secret(dir: &PathBuf, key: crate::crypto::SecretKey) -> Self {
        Self::open(dir.join("4.dat"), Some(key))
    }

    fn open(path: PathBuf, key: Option<crate::crypto::SecretKey>) -> Self {
        let creds = fs::read(&path)
            .ok()
            .and_then(|enc| match &key {
                Some(k) => crate::crypto::open(k, &enc),
                None => unprotect(&enc),
            })
            .and_then(|plain| serde_json::from_slice(&plain).ok())
            .unwrap_or_default();
        Self { path, key, creds }
    }

    pub fn rekey(&mut self, key: crate::crypto::SecretKey) {
        self.key = Some(key);
        self.save();
    }

    fn save(&self) {
        if self.creds.is_empty() {
            let _ = fs::remove_file(&self.path);
            return;
        }
        if let Ok(plain) = serde_json::to_vec(&self.creds) {
            let enc = match &self.key {
                Some(k) => Some(crate::crypto::seal(k, &plain)),
                None => protect(&plain),
            };
            if let Some(enc) = enc {
                let _ = fs::write(&self.path, enc);
            }
        }
    }

    /// Importe un export CSV. Renvoie le nombre d'identifiants ajoutés ou mis à jour.
    pub fn import_csv(&mut self, text: &str) -> Result<usize, String> {
        let rows = parse_csv(text);
        let header: Vec<String> = rows
            .first()
            .ok_or("fichier vide")?
            .iter()
            .map(|h| h.trim().to_lowercase())
            .collect();
        let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
        let (Some(iu), Some(in_), Some(ip)) = (
            col(&["url", "origin", "login_uri", "website"]),
            col(&["username", "login", "login_username", "email"]),
            col(&["password", "login_password"]),
        ) else {
            return Err("colonnes url / username / password introuvables".into());
        };
        let mut count = 0;
        for row in rows.iter().skip(1) {
            let get = |i: usize| row.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
            let (url, username, password) = (get(iu), get(in_), get(ip));
            if password.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
                continue;
            }
            let host = crate::shield::host(&url);
            match self
                .creds
                .iter_mut()
                .find(|c| crate::shield::host(&c.url) == host && c.username == username)
            {
                Some(existing) => existing.password = password,
                None => self.creds.push(Cred { url, username, password }),
            }
            count += 1;
        }
        self.save();
        Ok(count)
    }

    pub fn clear(&mut self) {
        self.creds.clear();
        self.save();
    }

    /// Script de remplissage automatique pour cette page, s'il y a des identifiants enregistrés.
    pub fn autofill_script(&self, page_url: &str) -> Option<String> {
        if !page_url.starts_with("https://") && !page_url.starts_with("http://") {
            return None;
        }
        let host = crate::shield::host(page_url);
        let creds: Vec<&Cred> = self
            .creds
            .iter()
            .filter(|c| {
                let h = crate::shield::host(&c.url);
                !h.is_empty() && (host == h || host.ends_with(&format!(".{h}")))
            })
            .collect();
        let first = creds.first()?;
        let data = serde_json::json!({ "u": first.username, "p": first.password });
        Some(AUTOFILL_JS.replace("__CREDS__", &data.to_string().replace('<', "\\u003c")))
    }
}

const AUTOFILL_JS: &str = r#"
(function (c) {
  if (window.top !== window) return;
  function visible(el) { return el && el.offsetParent !== null && !el.disabled && !el.readOnly; }
  function set(el, v) {
    if (!visible(el) || el.value) return;
    var d = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
    d.set.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }
  function fill() {
    var pw = Array.prototype.find.call(document.querySelectorAll('input[type=password]'), visible);
    var scope = (pw && pw.form) || document;
    var inputs = Array.prototype.slice.call(scope.querySelectorAll('input'));
    var user = null;
    if (pw) {
      for (var i = inputs.indexOf(pw) - 1; i >= 0; i--) {
        var t = (inputs[i].type || 'text').toLowerCase();
        if ((t === 'text' || t === 'email' || t === 'tel') && visible(inputs[i])) { user = inputs[i]; break; }
      }
    } else {
      // Connexion en deux étapes (ex. Google) : d'abord l'identifiant seul.
      user = Array.prototype.find.call(
        document.querySelectorAll('input[autocomplete~=username],input[type=email]'), visible);
    }
    if (user && c.u) set(user, c.u);
    if (pw) { set(pw, c.p); return true; }
    return false;
  }
  var n = 0;
  if (!fill()) { var t = setInterval(function () { if (fill() || ++n > 20) clearInterval(t); }, 500); }
})(__CREDS__);
"#;

/// Lecteur CSV : gère les guillemets, virgules et retours à la ligne dans les champs.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(ch),
            }
        } else {
            match ch {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(ch),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    rows
}

// ---------- Chiffrement (DPAPI : seul ton compte Windows peut déchiffrer) ----------

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let result = if encrypt {
            CryptProtectData(&input, windows::core::w!("CyberX"), None, None, None, 0, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut out)
        };
        result.ok()?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as _));
        Some(bytes)
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, true)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, false)
}

// Hors Windows (développement uniquement) : pas de chiffrement système disponible.
#[cfg(not(windows))]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}
