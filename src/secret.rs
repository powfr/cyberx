//! Espace secret : un second profil complet (historique, favoris, mots de passe,
//! onglets, connexions aux sites), ouvert en tapant le code secret dans la barre d'adresse.
//!
//! Dossier : %APPDATA%\CyberX\cache\
//!   id     : sel aléatoire (16 octets) + empreinte courte du code (2 octets)
//!   5.dat  : témoin chiffré servant à vérifier le code
//!   0-4.dat: données chiffrées (AES-256-GCM, clé dérivée du code)
//!   web\   : données des sites (cookies, connexions) propres à l'espace secret

use std::{fs, path::Path, path::PathBuf};

use crate::crypto::{self, SecretKey};

/// Code utilisé pour créer l'espace secret la toute première fois.
pub const DEFAULT_CODE: &str = "secretx";
const CHECK: &[u8] = b"cyberx-secret";

pub fn dir(base: &Path) -> PathBuf {
    base.join("cache")
}

pub fn valid_code(code: &str) -> bool {
    let n = code.chars().count();
    (4..=64).contains(&n) && !code.chars().any(char::is_whitespace)
}

/// Si `input` est le code secret, renvoie la clé de l'espace (en le créant au premier usage).
pub fn unlock(base: &Path, input: &str) -> Option<SecretKey> {
    if !valid_code(input) {
        return None;
    }
    let dir = dir(base);
    match fs::read(dir.join("id")) {
        Ok(id) if id.len() == 18 => {
            let (salt, short) = id.split_at(16);
            // Test rapide : écarte les recherches ordinaires sans calcul lent.
            if crypto::probe(input, salt) != short {
                return None;
            }
            let key = crypto::derive_key(input, salt);
            let check = fs::read(dir.join("5.dat")).ok()?;
            (crypto::open(&key, &check)?.as_slice() == CHECK).then_some(key)
        }
        _ if input == DEFAULT_CODE => {
            fs::create_dir_all(&dir).ok()?;
            Some(set_code(&dir, input))
        }
        _ => None,
    }
}

/// Enregistre un (nouveau) code et renvoie la clé correspondante.
/// Les données doivent ensuite être réécrites avec cette clé (voir `rekey`).
pub fn set_code(dir: &Path, code: &str) -> SecretKey {
    let salt = crypto::random::<16>();
    let key = crypto::derive_key(code, &salt);
    let mut id = salt.to_vec();
    id.extend_from_slice(&crypto::probe(code, &salt));
    let _ = fs::write(dir.join("id"), id);
    let _ = fs::write(dir.join("5.dat"), crypto::seal(&key, CHECK));
    key
}
