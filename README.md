# CyberX

Un navigateur web léger écrit en Rust, avec onglets, historique, favoris, navigation privée et thèmes.

## Fonctionnalités

- **Onglets** : ouvrir, fermer (clic molette aussi), passer de l'un à l'autre
- **Fenêtre façon Brave** : onglets intégrés dans la barre de titre, boutons ─ ▢ ✕, glisser la barre pour déplacer,
  double-clic pour agrandir, bords pour redimensionner
- **Vrai plein écran** pour les vidéos (YouTube, etc.), sortie avec Échap
- **Restauration** : les onglets normaux ouverts sont rouverts au prochain lancement (jamais les onglets privés)
- **Bouclier 🛡** : bloque pubs et traqueurs avec le moteur de Brave (`adblock-rust`) et les listes EasyList,
  EasyPrivacy et Liste FR, y compris dans les iframes. Anti-pub YouTube (masquage + saut automatique des pubs vidéo).
  Compteur par page, désactivation par site d'un clic (sans recharger), interrupteur global dans les paramètres
- **Navigation privée** : onglets 🕶 qui n'enregistrent pas d'historique, et dont les cookies et données sont effacés à la fermeture
- **Historique** : groupé par jour, recherche, suppression d'une entrée ou de tout l'historique
- **Favoris** : étoile ☆ dans la barre d'adresse, barre de favoris, page de gestion, raccourcis sur la page Nouvel onglet
- **6 thèmes** : Cyber Néon, Minuit, Clair, Matrix, Coucher de soleil, Océan
- **Moteur de recherche au choix** : DuckDuckGo (par défaut), Brave Search, Startpage, Qwant, Google
- **Barre d'adresse intelligente** : tape une adresse ou une recherche, avec suggestions depuis l'historique et les favoris

### Raccourcis clavier

| Raccourci | Action |
|---|---|
| `Ctrl+T` | Nouvel onglet |
| `Ctrl+Maj+N` | Nouvel onglet privé |
| `Ctrl+W` | Fermer l'onglet |
| `Ctrl+L` | Aller à la barre d'adresse |
| `Ctrl+D` | Ajouter / retirer des favoris |
| `Ctrl+H` | Historique |
| `Ctrl+Tab` | Onglet suivant |
| `F5` | Actualiser |
| `Échap` | Fermer le panneau |

## Installer (le plus simple)

Télécharge `cyberx.exe` dans la dernière version : https://github.com/powfr/cyberx/releases/latest
CyberX vérifie ensuite tout seul les nouvelles versions au lancement et propose de se mettre à jour.

## Publier une nouvelle version

Chaque modification envoyée sur `main` est compilée automatiquement (onglet *Actions*).
Pour publier : augmenter `version` dans `Cargo.toml`, puis créer une étiquette `vX.Y.Z`
(`git tag v0.4.0 && git push --tags`). GitHub publie alors `cyberx.exe` dans les Releases.

## Compiler soi-même sur Windows

1. **Installer Rust** : télécharge et lance `rustup-init.exe` depuis https://rustup.rs.
   Quand il te le propose, accepte d'installer les **Visual Studio C++ Build Tools** (obligatoire sous Windows).
2. **Ouvrir un terminal** (PowerShell) dans ce dossier `cyberx`.
3. **Compiler** :
   ```
   cargo build --release
   ```
   La première compilation prend quelques minutes (téléchargement des bibliothèques).
4. **Lancer** : `target\release\cyberx.exe`. Tu peux copier ce fichier où tu veux et créer un raccourci.

Pour tester rapidement pendant le développement : `cargo run`.

**Prérequis** : Windows 10 ou 11. Le moteur WebView2 est déjà installé sur ces versions de Windows ;
sinon, installe le « WebView2 Runtime » de Microsoft.

## Où sont mes données ?

`%APPDATA%\CyberX\` : `history.json`, `bookmarks.json`, `settings.json`, `session.json`
(onglets à rouvrir) et le dossier `filters\` (listes du bouclier).

## Structure du projet

```
src/main.rs      fenêtre, onglets, navigation, raccourcis
src/storage.rs   sauvegarde de l'historique, des favoris et des paramètres
src/themes.rs    thèmes et moteurs de recherche
src/shield.rs    bouclier : listes de filtres et moteur de blocage
src/shield_win.rs  interception des requêtes WebView2 (Windows)
src/ui.html      interface : onglets, barre d'adresse, panneaux
src/newtab.html  page Nouvel onglet
```

## Bon à savoir

L'interface et toute la logique sont en Rust, mais l'affichage des pages web utilise **WebView2**,
le moteur de Microsoft Edge (basé sur Chromium), via la bibliothèque `wry`. Recoder un moteur de rendu
web complet est un projet de plusieurs années (voir Servo) ; c'est ce qui permet à CyberX d'afficher
correctement YouTube, Netflix, etc. dès aujourd'hui.

Les pubs YouTube sont les plus difficiles à bloquer : YouTube change souvent sa technique.

Pas encore inclus : extensions, gestionnaire de téléchargements dédié
(les téléchargements passent par la fenêtre native de WebView2), synchronisation.

## En cas d'erreur de compilation « mismatched types » avec `webview2_com` ou `windows`

Les versions de `webview2-com` et `windows` dans `Cargo.toml` doivent être exactement celles qu'utilise `wry`.
Lance `cargo tree -i webview2-com` et `cargo tree -i windows`, puis mets les mêmes numéros dans `Cargo.toml`.
