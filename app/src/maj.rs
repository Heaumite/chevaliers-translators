//! Mises à jour depuis GitHub (0.16 ; 0.16.1 : installées sur demande du joueur).
//!
//! L'application regarde la dernière version publiée sur GitHub (Releases du dépôt), 20 s après
//! son lancement puis toutes les 30 minutes (ou tout de suite par le menu de l'icône). Une version
//! plus récente est téléchargée en fond : le paquet (`ChevaliersTranslators-X.Y.Z.zip`) doit avoir
//! exactement l'empreinte SHA-256 publiée avec lui (`.zip.sha256`), sinon il est refusé ; son
//! exécutable en est extrait.
//!
//! Le joueur est alors prévenu : la fenêtre de l'application montre « Mise à jour X disponible »,
//! un bouton pour l'installer et la note de mise à jour (le texte de la version sur GitHub). WoW au
//! premier plan : une bulle près de l'horloge, et la fenêtre s'ouvre dès que WoW n'est plus devant.
//! Au clic, l'exécutable en cours est renommé (`.exe.ancien`, Windows le permet pendant qu'il
//! tourne), le nouveau prend sa place, il est lancé et l'ancien s'arrête. Au démarrage, la nouvelle
//! version pose son addon dans toutes les versions de WoW trouvées, vérifié fichier par fichier, et
//! remplace les copies de l'exécutable rangées dans les dossiers de l'addon.
//!
//! Fenêtre de l'application (processus à part) et application : un petit fichier d'état
//! (`maj/etat.json`) et un fichier de demande d'installation (`maj/installer.demande`).
//! Menu de l'icône : « Rechercher une mise à jour », « Rechercher les mises à jour
//! automatiquement » (coupable).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Dépôt des versions publiées.
pub const REPO: &str = "Heaumite/chevaliers-translators";
/// Recherche d'une nouvelle version.
pub const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
const FIRST_CHECK: Duration = Duration::from_secs(20);
/// Nom de l'exécutable dans le paquet.
const EXE_IN_ZIP: &str = "ChevaliersTranslators/ChevaliersTranslators.exe";

/// Version publiée sur GitHub.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub zip_name: String,
    pub zip_url: String,
    pub size: u64,
    pub sha_url: String,
    /// Note de mise à jour (texte de la version sur GitHub, en Markdown).
    pub notes: String,
}

/// « v0.15.2 », « 0.15.2 » -> (0, 15, 2).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let s = s.split(['-', '+']).next()?;
    let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// Dernière version publiée (réponse de l'API de GitHub), avec son paquet et son empreinte.
/// Brouillon, préversion ou paquet pas encore déposé : rien.
pub fn release_from_json(v: &Value) -> Option<Release> {
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = v["tag_name"].as_str()?.trim().trim_start_matches('v').to_string();
    parse_version(&version)?;
    let zip_name = format!("ChevaliersTranslators-{version}.zip");
    let sha_name = format!("{zip_name}.sha256");
    let assets = v["assets"].as_array()?;
    let find = |name: &str| assets.iter().find(|a| a["name"].as_str() == Some(name));
    let (zip, sha) = (find(&zip_name)?, find(&sha_name)?);
    Some(Release {
        zip_url: zip["browser_download_url"].as_str()?.to_string(),
        size: zip["size"].as_u64().unwrap_or(0),
        sha_url: sha["browser_download_url"].as_str()?.to_string(),
        notes: v["body"].as_str().unwrap_or("").replace("\r\n", "\n").trim().to_string(),
        version,
        zip_name,
    })
}

/// Empreinte lue dans le fichier `.sha256` (« <hex>  <nom> », format de sha256sum).
pub fn sha_from_text(text: &str, zip_name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let name = parts.next().map(|n| n.trim_start_matches('*'));
        (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) && name.is_none_or(|n| n == zip_name))
            .then(|| hex.to_ascii_lowercase())
    })
}

/// Exécutable compilé sur place (développement) : jamais mis à jour seul.
pub fn is_dev_build(exe: &Path) -> bool {
    exe.components().any(|c| {
        let n = c.as_os_str().to_string_lossy().to_lowercase();
        n == "target" || n == "target-win"
    })
}

fn off_file() -> PathBuf {
    crate::fonts::cache_root().join("maj-auto-coupee.txt")
}

/// Mises à jour automatiques actives (par défaut) ; coupées par le menu de l'icône.
pub fn auto_enabled() -> bool {
    !off_file().exists()
}

pub fn set_auto(on: bool) {
    if on {
        let _ = std::fs::remove_file(off_file());
    } else {
        let _ = std::fs::create_dir_all(crate::fonts::cache_root());
        let _ = std::fs::write(off_file(), "Mises à jour automatiques coupées depuis le menu de l'icône.\n");
    }
}

// --- Paquet zip -----------------------------------------------------------------------------------

/// Extrait un fichier d'un zip (stocké ou compressé « deflate », sans zip64), contrôle CRC compris.
pub fn unzip_one(zip: &[u8], wanted: &str) -> Result<Vec<u8>, String> {
    let u16_at = |o: usize| -> Result<usize, String> {
        zip.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| "zip tronqué".to_string())
    };
    let u32_at = |o: usize| -> Result<u32, String> {
        zip.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "zip tronqué".to_string())
    };
    // Fin du répertoire central : cherchée depuis la fin (commentaire éventuel).
    let start = zip.len().saturating_sub(22 + 65535);
    let eocd = (start..zip.len().saturating_sub(21))
        .rev()
        .find(|&o| zip[o..o + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("paquet illisible (pas un zip)")?;
    let count = u16_at(eocd + 10)?;
    let mut o = u32_at(eocd + 16)? as usize;
    for _ in 0..count {
        if u32_at(o)? != 0x0201_4b50 {
            return Err("répertoire du zip illisible".into());
        }
        let method = u16_at(o + 10)?;
        let crc = u32_at(o + 16)?;
        let csize = u32_at(o + 20)? as usize;
        let usize_ = u32_at(o + 24)? as usize;
        let (n, e, c) = (u16_at(o + 28)?, u16_at(o + 30)?, u16_at(o + 32)?);
        let local = u32_at(o + 42)? as usize;
        let name = String::from_utf8_lossy(zip.get(o + 46..o + 46 + n).ok_or("zip tronqué")?).replace('\\', "/");
        o += 46 + n + e + c;
        if name != wanted {
            continue;
        }
        if u32_at(local)? != 0x0403_4b50 {
            return Err("entrée du zip illisible".into());
        }
        let data_at = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let raw = zip.get(data_at..data_at + csize).ok_or("zip tronqué")?;
        let out = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut v).map_err(|e| format!("zip : {e}"))?;
                v
            }
            m => return Err(format!("compression {m} du zip non prise en charge")),
        };
        if out.len() != usize_ || crate::proto::crc32(&out) != crc {
            return Err(format!("{wanted} abîmé dans le paquet"));
        }
        return Ok(out);
    }
    Err(format!("{wanted} absent du paquet"))
}

/// Exécutable Windows plausible (en-tête « MZ », taille d'une vraie application).
pub fn looks_like_exe(data: &[u8]) -> bool {
    data.len() > 500_000 && data.starts_with(b"MZ")
}

// --- Remplacement de l'exécutable ---------------------------------------------------------------

fn old_path(current: &Path) -> PathBuf {
    let mut s = current.as_os_str().to_os_string();
    s.push(".ancien");
    PathBuf::from(s)
}

/// Met le nouvel exécutable à la place de celui qui tourne : l'actuel est renommé (`.ancien`),
/// le nouveau copié à son nom. En cas d'échec, l'actuel reprend sa place.
pub fn apply(new_exe: &Path, current: &Path) -> Result<(), String> {
    let old = old_path(current);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(current, &old).map_err(|e| format!("{} : {e}", current.display()))?;
    if let Err(e) = std::fs::copy(new_exe, current) {
        let _ = std::fs::rename(&old, current);
        return Err(format!("{} : {e}", current.display()));
    }
    Ok(())
}

/// Ancien exécutable laissé par une mise à jour : supprimé au lancement suivant.
pub fn cleanup_old(current: &Path) {
    let _ = std::fs::remove_file(old_path(current));
}

// --- Recherche et téléchargement (fil à part) --------------------------------------------------

fn curl() -> String {
    std::env::var("CT_CURL").unwrap_or_else(|_| if cfg!(windows) { "curl.exe" } else { "curl" }.into())
}

fn curl_get(url: &str, out: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new(curl());
    crate::quiet(&mut cmd).args(["-sSfL", "--retry", "3", "--max-time", "300", "-H", "Accept: application/vnd.github+json"]);
    if let Some(p) = out {
        cmd.arg("-o").arg(p);
    }
    let r = cmd.arg(url).output().map_err(|e| format!("curl introuvable : {e}"))?;
    if !r.status.success() {
        return Err(format!("{} ({})", String::from_utf8_lossy(&r.stderr).trim(), r.status));
    }
    Ok(r.stdout)
}

/// Dernière version publiée sur GitHub.
pub fn latest() -> Result<Option<Release>, String> {
    let body = curl_get(&format!("https://api.github.com/repos/{REPO}/releases/latest"), None)?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("réponse de GitHub illisible : {e}"))?;
    Ok(release_from_json(&v))
}

/// Télécharge et vérifie le paquet ; renvoie le nouvel exécutable, prêt à être mis en place.
pub fn download(rel: &Release) -> Result<PathBuf, String> {
    let dir = crate::fonts::cache_root().join("maj").join(&rel.version);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let sha_text = String::from_utf8_lossy(&curl_get(&rel.sha_url, None)?).to_string();
    let expected = sha_from_text(&sha_text, &rel.zip_name).ok_or("empreinte SHA-256 illisible")?;
    let zip_path = dir.join(&rel.zip_name);
    curl_get(&rel.zip_url, Some(&zip_path))?;
    let zip = std::fs::read(&zip_path).map_err(|e| format!("{} : {e}", zip_path.display()))?;
    let got = crate::sha256::sha256_hex(&zip);
    if got != expected {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!("empreinte du paquet différente de celle publiée ({got} au lieu de {expected}) : paquet refusé"));
    }
    let exe = unzip_one(&zip, EXE_IN_ZIP)?;
    if !looks_like_exe(&exe) {
        return Err("le paquet ne contient pas d'exécutable valable".into());
    }
    let out = dir.join("ChevaliersTranslators.exe");
    std::fs::write(&out, &exe).map_err(|e| format!("{} : {e}", out.display()))?;
    let _ = std::fs::remove_file(&zip_path);
    Ok(out)
}

/// Ce que le fil des mises à jour signale à l'application (bulles de l'icône).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Recherche demandée par le joueur : déjà la dernière version.
    UpToDate(String),
    /// Nouvelle version trouvée, téléchargement en cours (recherche demandée par le joueur).
    Found(String),
    /// Nouvelle version téléchargée et vérifiée : le joueur est prévenu (fenêtre, bouton, note).
    Ready(String),
    Problem(String),
}

#[derive(Default)]
struct State {
    ready: Option<(String, PathBuf)>,
    /// Version qui n'a pas pu être installée : plus essayée avant le prochain lancement.
    skip: Option<String>,
    events: Vec<Event>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static CHECK_NOW: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// Recherche tout de suite (menu de l'icône).
pub fn check_now() {
    CHECK_NOW.store(true, Ordering::Relaxed);
}

pub fn take_events() -> Vec<Event> {
    with(|s| std::mem::take(&mut s.events))
}

/// Nouvelle version prête à installer : (version, exécutable).
pub fn ready() -> Option<(String, PathBuf)> {
    with(|s| s.ready.clone())
}

/// Installation impossible : la version prête est oubliée (nouvel essai au prochain lancement).
pub fn forget_ready() {
    with(|s| s.skip = s.ready.take().map(|(v, _)| v));
}

// --- État partagé avec la fenêtre de l'application ---------------------------------------------

fn dir() -> PathBuf {
    crate::fonts::cache_root().join("maj")
}

fn state_file() -> PathBuf {
    dir().join("etat.json")
}

fn installed_file() -> PathBuf {
    dir().join("installee.json")
}

fn request_file() -> PathBuf {
    dir().join("installer.demande")
}

/// État de la mise à jour vu par la fenêtre : version, note, étape (« telechargement »,
/// « prete », « installation », « erreur »), message.
pub fn write_state(version: &str, notes: &str, step: &str, message: &str) {
    let _ = std::fs::create_dir_all(dir());
    let v = serde_json::json!({ "version": version, "notes": notes, "etat": step, "message": message });
    let _ = std::fs::write(state_file(), v.to_string());
}

/// Nouvelle étape de la mise à jour, sa note gardée (montrée encore après l'installation).
pub fn set_step(version: &str, step: &str, message: &str) {
    let notes = std::fs::read(state_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| v["version"].as_str() == Some(version))
        .and_then(|v| v["notes"].as_str().map(str::to_string))
        .unwrap_or_default();
    write_state(version, &notes, step, message);
}

/// Mise à jour à montrer dans la fenêtre : seulement une version plus récente que celle-ci.
pub fn read_state(current: &str) -> Option<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(state_file()).ok()?).ok()?;
    is_newer(v["version"].as_str()?, current).then_some(v)
}

/// Nouveautés de la version qui vient d'être installée (montrées une fois dans la fenêtre).
pub fn read_installed(current: &str) -> Option<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(installed_file()).ok()?).ok()?;
    (v["version"].as_str()? == current).then_some(v)
}

/// Nouveautés lues par le joueur.
pub fn dismiss_installed() {
    let _ = std::fs::remove_file(installed_file());
}

/// Mise à jour proposée au joueur et pas encore installée (« prete », ou « erreur » à réessayer) :
/// sa version. L'encadré de la fenêtre et l'entrée du menu de l'icône restent jusqu'à son clic.
pub fn pending(current: &str) -> Option<String> {
    let v = read_state(current)?;
    matches!(v["etat"].as_str(), Some("prete" | "erreur")).then(|| v["version"].as_str().unwrap_or_default().to_string())
}

/// Au lancement (0.16.1) : une mise à jour proposée reste proposée (fenêtre, menu de l'icône)
/// jusqu'au clic du joueur ; un téléchargement ou une installation interrompus par l'arrêt de
/// l'application, ou un état d'une version qui n'est plus nouvelle, sont oubliés (la recherche
/// les refait).
pub fn tidy_state(current: &str) {
    tidy_state_in(&state_file(), current);
}

fn tidy_state_in(file: &std::path::Path, current: &str) {
    let keep = std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| {
            v["version"].as_str().is_some_and(|ver| is_newer(ver, current)) && matches!(v["etat"].as_str(), Some("prete" | "erreur"))
        });
    if !keep {
        let _ = std::fs::remove_file(file);
    }
}

/// Au premier lancement de la version installée : l'état de la mise à jour devient « nouveautés ».
pub fn mark_installed(current: &str) {
    if let Ok(Some(v)) = std::fs::read(state_file()).map(|b| serde_json::from_slice::<Value>(&b).ok()) {
        if v["version"].as_str() == Some(current) {
            let _ = std::fs::write(installed_file(), v.to_string());
        }
    }
    let _ = std::fs::remove_file(state_file());
}

/// Bouton « Installer la mise à jour » de la fenêtre.
pub fn request_install() -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(request_file(), b"1").map_err(|e| e.to_string())
}

/// Installation demandée par le joueur depuis la fenêtre (lue une fois).
pub fn take_install_request() -> bool {
    std::fs::remove_file(request_file()).is_ok()
}

/// Lance la recherche en fond.
pub fn start(current: &'static str, exe: PathBuf) {
    std::thread::spawn(move || {
        let dev = is_dev_build(&exe);
        let started = Instant::now();
        let mut last: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let forced = CHECK_NOW.swap(false, Ordering::Relaxed);
            let due = match last {
                None => started.elapsed() >= FIRST_CHECK,
                Some(t) => t.elapsed() >= CHECK_EVERY,
            };
            if !forced && !(due && auto_enabled()) {
                continue;
            }
            last = Some(Instant::now());
            match latest() {
                Ok(Some(rel)) if is_newer(&rel.version, current) => {
                    let known = with(|s| s.ready.as_ref().is_some_and(|(v, _)| *v == rel.version) || s.skip.as_deref() == Some(rel.version.as_str()));
                    if known {
                        if forced {
                            with(|s| s.events.push(Event::Ready(rel.version.clone())));
                        }
                        continue;
                    }
                    crate::say!(
                        "[mise à jour] version {} publiée sur GitHub : téléchargement{}",
                        rel.version,
                        if dev { " (version de développement : installation seulement par le bouton)" } else { "" }
                    );
                    // Déjà proposée avant un redémarrage : l'encadré garde son bouton pendant le
                    // nouveau téléchargement (l'exécutable vérifié n'est pas gardé d'une session à
                    // l'autre).
                    if pending(current).as_deref() != Some(rel.version.as_str()) {
                        write_state(&rel.version, &rel.notes, "telechargement", "");
                    }
                    if forced {
                        with(|s| s.events.push(Event::Found(rel.version.clone())));
                    }
                    match download(&rel) {
                        Ok(path) => {
                            crate::say!("[mise à jour] {} téléchargée et vérifiée (SHA-256) : {}", rel.version, path.display());
                            write_state(&rel.version, &rel.notes, "prete", "");
                            with(|s| {
                                s.ready = Some((rel.version.clone(), path));
                                s.events.push(Event::Ready(rel.version.clone()));
                            });
                        }
                        Err(e) => {
                            crate::say!("[mise à jour] {} : {e}", rel.version);
                            write_state(&rel.version, &rel.notes, "erreur", &e);
                            with(|s| s.events.push(Event::Problem(format!("version {} : {e}", rel.version))));
                        }
                    }
                }
                Ok(_) => {
                    if forced {
                        with(|s| s.events.push(Event::UpToDate(current.to_string())));
                    }
                }
                Err(e) => {
                    crate::say!("[mise à jour] recherche impossible : {e}");
                    if forced {
                        with(|s| s.events.push(Event::Problem(format!("recherche impossible : {e}"))));
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposed_update_kept_until_clicked() {
        let d = std::env::temp_dir().join(format!("ct-maj-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("etat.json");
        let put = |ver: &str, step: &str| std::fs::write(&f, json!({ "version": ver, "notes": "- x", "etat": step, "message": "" }).to_string()).unwrap();
        // Proposée (ou à réessayer) : gardée d'un lancement à l'autre.
        for step in ["prete", "erreur"] {
            put("0.16.2", step);
            tidy_state_in(&f, "0.16.1");
            assert!(f.exists(), "{step} oubliée");
        }
        // Téléchargement ou installation interrompus, version plus nouvelle : oubliés.
        for step in ["telechargement", "installation"] {
            put("0.16.2", step);
            tidy_state_in(&f, "0.16.1");
            assert!(!f.exists(), "{step} gardée");
        }
        put("0.16.1", "prete");
        tidy_state_in(&f, "0.16.1");
        assert!(!f.exists(), "version déjà installée gardée");
        let _ = std::fs::remove_dir_all(&d);
    }
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.15.2"), Some((0, 15, 2)));
        assert_eq!(parse_version("1.0.0-rc1"), Some((1, 0, 0)));
        assert_eq!(parse_version("0.15"), None);
        assert!(is_newer("v0.15.10", "0.15.9"));
        assert!(is_newer("0.16.0", "0.15.1"));
        assert!(!is_newer("0.15.1", "0.15.1"));
        assert!(!is_newer("0.14.9", "0.15.1"));
        assert!(!is_newer("nimporte", "0.15.1"));
    }

    #[test]
    fn latest_release_needs_its_package_and_checksum() {
        let asset = |name: &str| json!({ "name": name, "size": 2_400_000, "browser_download_url": format!("https://github.com/{REPO}/releases/download/v0.16.0/{name}") });
        let full = json!({ "tag_name": "v0.16.0", "draft": false, "prerelease": false,
            "assets": [asset("ChevaliersTranslators-0.16.0.zip"), asset("ChevaliersTranslators-0.16.0.zip.sha256")] });
        let r = release_from_json(&full).unwrap();
        assert_eq!(r.version, "0.16.0");
        assert!(r.zip_url.ends_with("/v0.16.0/ChevaliersTranslators-0.16.0.zip") && r.sha_url.ends_with(".zip.sha256"));
        assert_eq!(r.size, 2_400_000);
        let mut with_notes = full.clone();
        with_notes["body"] = json!("- **Bulles** traduites\r\n- Envoyer");
        assert_eq!(release_from_json(&with_notes).unwrap().notes, "- **Bulles** traduites\n- Envoyer");
        // Paquet pas encore déposé (publication en cours), préversion, brouillon : rien.
        let partial = json!({ "tag_name": "v0.16.0", "assets": [asset("ChevaliersTranslators-0.16.0.zip")] });
        assert!(release_from_json(&partial).is_none());
        let mut pre = full.clone();
        pre["prerelease"] = json!(true);
        assert!(release_from_json(&pre).is_none());
        let sha = "3f".repeat(32);
        assert_eq!(sha_from_text(&format!("{sha}  ChevaliersTranslators-0.16.0.zip\n"), "ChevaliersTranslators-0.16.0.zip"), Some(sha.clone()));
        assert_eq!(sha_from_text(&format!("{}  autre.zip", sha.to_uppercase()), "ChevaliersTranslators-0.16.0.zip"), None);
        assert_eq!(sha_from_text("pas une empreinte", "x.zip"), None);
    }

    /// Zip minimal : une entrée stockée, une compressée (comme ZipFile de .NET).
    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in entries {
            let body = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let crc = crate::proto::crc32(data);
            let local = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&local.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn the_new_exe_comes_out_of_the_package_and_takes_the_place_of_the_running_one() {
        let mut exe = b"MZ".to_vec();
        exe.extend((0..600_000u32).map(|i| (i * 7 % 251) as u8));
        let zip = zip_of(&[
            ("ChevaliersTranslators/Core.lua", b"-- addon\n", false),
            (EXE_IN_ZIP, &exe, true),
            ("ChevaliersTranslators/LICENSE.txt", b"GPL", false),
        ]);
        let got = unzip_one(&zip, EXE_IN_ZIP).unwrap();
        assert_eq!(got, exe);
        assert!(looks_like_exe(&got));
        assert_eq!(unzip_one(&zip, "ChevaliersTranslators/Core.lua").unwrap(), b"-- addon\n");
        assert!(unzip_one(&zip, "absent.exe").is_err());
        assert!(unzip_one(b"pas un zip du tout, mais assez long pour la recherche", EXE_IN_ZIP).is_err());
        // Contenu abîmé : refusé (CRC).
        let mut bad = zip.clone();
        let i = bad.windows(4).position(|w| w == b"-- a").unwrap();
        bad[i] = b'#';
        assert!(unzip_one(&bad, "ChevaliersTranslators/Core.lua").is_err());
        // Mise en place : l'actuel est renommé, le nouveau prend son nom ; l'ancien est retiré au
        // lancement suivant.
        let dir = std::env::temp_dir().join(format!("ct-maj-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("ChevaliersTranslators.exe");
        let new = dir.join("nouveau.exe");
        std::fs::write(&current, b"MZ ancien").unwrap();
        std::fs::write(&new, &exe).unwrap();
        apply(&new, &current).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), exe);
        assert_eq!(std::fs::read(old_path(&current)).unwrap(), b"MZ ancien");
        cleanup_old(&current);
        assert!(!old_path(&current).exists());
        // Échec de la copie (nouvel exécutable absent) : l'actuel reste en place.
        std::fs::remove_file(&new).unwrap();
        assert!(apply(&new, &current).is_err());
        assert_eq!(std::fs::read(&current).unwrap(), exe);
        std::fs::remove_dir_all(&dir).ok();
        // Copies de développement : jamais mises à jour seules.
        assert!(is_dev_build(Path::new(r"C:\Dev\ChevaliersTranslators\chevaliers-translators\app\target-win\release\ChevaliersTranslators.exe".replace('\\', "/").as_str())));
        assert!(!is_dev_build(Path::new("/jeux/World of Warcraft/_retail_/Interface/AddOns/ChevaliersTranslators/ChevaliersTranslators.exe")));
    }
}
