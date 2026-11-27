//! Chiffrement de l'espace secret : clé dérivée du code secret (PBKDF2-SHA256),
//! données chiffrées et authentifiées avec AES-256-GCM.
//! Sans le code, les fichiers sont illisibles et impossibles à modifier sans que ça se voie.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use sha2::{Digest, Sha256};

pub type SecretKey = [u8; 32];

/// Nombre d'itérations PBKDF2 : rend très lent le test de codes au hasard.
const ROUNDS: u32 = 400_000;

pub fn random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).expect("générateur aléatoire indisponible");
    buf
}

pub fn derive_key(code: &str, salt: &[u8]) -> SecretKey {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(code.as_bytes(), salt, ROUNDS, &mut key);
    key
}

/// Petite empreinte (2 octets) du code : permet d'écarter instantanément les saisies
/// ordinaires sans lancer la dérivation lente. Elle ne suffit pas à retrouver le code.
pub fn probe(code: &str, salt: &[u8]) -> [u8; 2] {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(code.as_bytes());
    let d = h.finalize();
    [d[0], d[1]]
}

/// Chiffre : nonce (12 octets) suivi des données chiffrées.
pub fn seal(key: &SecretKey, plain: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = random::<12>();
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(Nonce::from_slice(&nonce), plain).expect("chiffrement"));
    out
}

/// Déchiffre ; `None` si la clé est mauvaise ou si le fichier a été modifié.
pub fn open(key: &SecretKey, data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 {
        return None;
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher.decrypt(Nonce::from_slice(&data[..12]), &data[12..]).ok()
}
