//! Nouvelles versions des modèles publiées par leurs éditeurs (0.16.6). Les fichiers viennent de
//! Hugging Face (branche principale du dépôt de chaque éditeur) : quand l'éditeur y remplace un
//! fichier (entraînement poursuivi, conversion refaite), son empreinte SHA-256 publiée change.
//! L'application la compare, une fois par jour et WoW fermé, à celle du fichier installé (calculée
//! une fois, gardée tant que le fichier ne change pas) ; une nouvelle version est proposée dans la
//! fenêtre des modèles (« Mettre à jour »). Téléchargée à côté (`<fichier>.nouveau`) et vérifiée
//! (taille et empreinte publiées), elle remplace l'ancienne quand aucun serveur ne l'utilise : au
//! lancement de l'application ou quand WoW est fermé. Les nouveaux modèles (autres familles,
//! nouvelles générations) arrivent, eux, avec les mises à jour de l'application.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use crate::catalog::{self, Model, Progress};

/// Dépôt et chemin du fichier dans une adresse Hugging Face `…/<dépôt>/resolve/<révision>/<chemin>`.
pub fn hf_parts(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://huggingface.co/")?;
    let (repo, path) = rest.split_once("/resolve/")?;
    let (_rev, file) = path.split_once('/')?;
    Some((repo.to_string(), file.to_string()))
}

/// Version publiée d'un fichier : taille et empreinte SHA-256.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub size: u64,
    pub oid: String,
}

/// Réponse de l'API (liste d'un dossier du dépôt) : la version publiée de `file`.
pub fn parse_tree(json: &str, file: &str) -> Option<Remote> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.as_array()?.iter().find(|e| e["path"].as_str() == Some(file)).and_then(|e| {
        let lfs = &e["lfs"];
        let oid = lfs["oid"].as_str()?.to_string();
        let size = lfs["size"].as_u64().or_else(|| e["size"].as_u64())?;
        (oid.len() == 64).then_some(Remote { size, oid })
    })
}

/// Version publiée aujourd'hui (réseau, 20 s au plus).
pub fn remote(url: &str) -> Result<Remote, String> {
    let (repo, file) = hf_parts(url).ok_or("adresse hors de Hugging Face")?;
    let api = match file.rsplit_once('/') {
        Some((d, _)) => format!("https://huggingface.co/api/models/{repo}/tree/main/{d}"),
        None => format!("https://huggingface.co/api/models/{repo}/tree/main"),
    };
    let out = crate::quiet(&mut Command::new(catalog::curl()))
        .args(["-sL", "--max-time", "20", &api])
        .output()
        .map_err(|e| format!("curl introuvable : {e}"))?;
    parse_tree(&String::from_utf8_lossy(&out.stdout), &file).ok_or_else(|| format!("{file} absent de la réponse de Hugging Face"))
}

fn cache_path() -> PathBuf {
    crate::fonts::cache_root().join("modeles-versions.json")
}

fn load() -> Value {
    std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

fn save(v: &Value) {
    let _ = std::fs::write(cache_path(), serde_json::to_vec_pretty(v).unwrap_or_default());
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Taille publiée retenue pour un fichier (dernière vérification ou dernier téléchargement) : un
/// fichier de cette taille est installé, même si le catalogue en donne une autre.
pub fn known_size(file: &str) -> Option<u64> {
    load()[file]["remote"]["size"].as_u64()
}

/// Taille publiée aujourd'hui par l'éditeur (avant un premier téléchargement) ; retenue. `None`
/// hors ligne ou pendant les tests (faux curl).
pub fn published_size(m: &Model) -> Option<u64> {
    if std::env::var_os("CT_CURL").is_some() {
        return None;
    }
    let r = remote(m.url).ok()?;
    let mut c = load();
    c[m.file]["remote"] = json!({ "size": r.size, "oid": r.oid, "at": now_secs() });
    save(&c);
    Some(r.size)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Empreinte du fichier installé : gardée tant que sa taille et sa date ne changent pas ; sinon
/// calculée (plusieurs secondes pour quelques gigaoctets).
fn local_oid(path: &Path, file: &str, cache: &mut Value) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    let mt = mtime(path);
    let l = &cache[file]["local"];
    if l["size"].as_u64() == Some(size) && l["mtime"].as_u64() == Some(mt) {
        if let Some(o) = l["oid"].as_str() {
            return Some(o.to_string());
        }
    }
    let oid = crate::sha256::file_hex(path).ok()?;
    cache[file]["local"] = json!({ "size": size, "mtime": mt, "oid": oid });
    Some(oid)
}

/// Nouvelle version d'un modèle installé.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub model: &'static Model,
    pub size: u64,
    /// Déjà téléchargée et vérifiée : remplacera l'ancienne quand aucun serveur ne l'utilise.
    pub ready: bool,
}

fn new_path(dir: &Path, m: &Model) -> PathBuf {
    dir.join(format!("{}.nouveau", m.file))
}

/// Nouvelles versions connues (dernière vérification), sans réseau.
pub fn available(dir: &Path) -> Vec<Update> {
    let c = load();
    catalog::all()
        .filter(|m| dir.join(m.file).exists())
        .filter_map(|m| {
            let (l, r) = (c[m.file]["local"]["oid"].as_str()?, &c[m.file]["remote"]);
            let (ro, rs) = (r["oid"].as_str()?, r["size"].as_u64()?);
            (l != ro).then(|| Update { model: m, size: rs, ready: std::fs::metadata(new_path(dir, m)).map(|x| x.len()).ok() == Some(rs) })
        })
        .collect()
}

/// Vérification (réseau, empreintes des fichiers installés) : les nouvelles versions publiées.
pub fn check(dir: &Path) -> Result<Vec<Update>, String> {
    let mut c = load();
    let mut errors = Vec::new();
    for m in catalog::all().filter(|m| dir.join(m.file).exists()) {
        match remote(m.url) {
            Ok(r) => c[m.file]["remote"] = json!({ "size": r.size, "oid": r.oid, "at": now_secs() }),
            Err(e) => errors.push(format!("{} : {e}", m.name)),
        }
        local_oid(&dir.join(m.file), m.file, &mut c);
    }
    c["_verifie"] = json!(now_secs());
    save(&c);
    if !errors.is_empty() && available(dir).is_empty() {
        return Err(errors.join(" ; "));
    }
    Ok(available(dir))
}

/// Dernière vérification (secondes depuis 1970), pour n'en faire qu'une par jour.
pub fn last_check() -> u64 {
    load()["_verifie"].as_u64().unwrap_or(0)
}

/// Télécharge la nouvelle version à côté de l'ancienne et vérifie son empreinte publiée.
pub fn download_update(m: &Model, dir: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(&Progress)) -> Result<(), String> {
    let r = remote(m.url)?;
    let dest = new_path(dir, m);
    catalog::download_file(m.url, &dest, r.size, m.name, cancel, report)?;
    let got = crate::sha256::file_hex(&dest).map_err(|e| e.to_string())?;
    if got != r.oid {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("{} : nouvelle version reçue mais empreinte différente de celle publiée : effacée", m.name));
    }
    let mut c = load();
    c[m.file]["remote"] = json!({ "size": r.size, "oid": r.oid, "at": now_secs() });
    c[m.file]["nouveau"] = json!(r.oid);
    save(&c);
    Ok(())
}

/// Nouvelles versions téléchargées mises en place (aucun serveur ne doit utiliser l'ancienne : au
/// lancement, ou WoW fermé). Les noms des modèles remplacés.
pub fn apply_pending(dir: &Path) -> Vec<String> {
    let mut c = load();
    let mut done = Vec::new();
    for m in catalog::all() {
        let new = new_path(dir, m);
        let Some(oid) = c[m.file]["nouveau"].as_str().map(str::to_string) else { continue };
        let size = c[m.file]["remote"]["size"].as_u64().unwrap_or(0);
        if std::fs::metadata(&new).map(|x| x.len()).ok() != Some(size) {
            continue;
        }
        let old = dir.join(m.file);
        // Ancienne version encore ouverte (serveur lancé) : réessayé plus tard.
        if old.exists() && std::fs::remove_file(&old).is_err() {
            continue;
        }
        if std::fs::rename(&new, &old).is_ok() {
            c[m.file]["local"] = json!({ "size": size, "mtime": mtime(&old), "oid": oid });
            if let Some(o) = c[m.file].as_object_mut() {
                o.remove("nouveau");
            }
            done.push(m.name.to_string());
        }
    }
    if !done.is_empty() {
        save(&c);
    }
    done
}

/// Nouvelles versions trouvées par la vérification de fond, à annoncer (noms des modèles).
static NEWS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Annonces à faire (icône, jeu) ; vidées.
pub fn take_news() -> Vec<String> {
    NEWS.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
}

/// Vérification de fond : 3 min après le lancement, puis toutes les 30 min ; une par jour au plus,
/// seulement WoW fermé (le calcul des empreintes lit les fichiers des modèles). `wow_open` : un
/// jeu est ouvert.
pub fn start_checker(dir: PathBuf, wow_open: fn() -> bool) {
    std::thread::spawn(move || {
        let mut announced: Vec<(String, u64)> = Vec::new();
        std::thread::sleep(std::time::Duration::from_secs(180));
        loop {
            if !wow_open() && now_secs().saturating_sub(last_check()) >= 24 * 3600 {
                match check(&dir) {
                    Ok(ups) => {
                        crate::say!("[modèles] versions publiées vérifiées : {}", if ups.is_empty() { "tout est à jour".to_string() } else {
                            ups.iter().map(|u| format!("{} ({})", u.model.name, catalog::go(u.size))).collect::<Vec<_>>().join(", ")
                        });
                        for u in ups {
                            let key = (u.model.file.to_string(), u.size);
                            if !u.ready && !announced.contains(&key) {
                                announced.push(key);
                                if let Ok(mut n) = NEWS.lock() {
                                    n.push(u.model.name.to_string());
                                }
                            }
                        }
                    }
                    Err(e) => crate::say!("[modèles] vérification des versions publiées impossible : {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1800));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hugging_face_tree_gives_the_published_version() {
        assert_eq!(
            hf_parts("https://huggingface.co/BSC-LT/salamandraTA-7B-instruct-GGUF/resolve/main/salamandraTA_7B_v3_q4_k_m.gguf"),
            Some(("BSC-LT/salamandraTA-7B-instruct-GGUF".into(), "salamandraTA_7B_v3_q4_k_m.gguf".into()))
        );
        assert_eq!(hf_parts("https://exemple.org/x.gguf"), None);
        let oid = "1cade17f491ea46a686dbee51fbd52442e0f001f102380c3b9d66b4a77f84093";
        let tree = format!(
            r#"[{{"type":"file","path":"README.md","size":4092}},
               {{"type":"file","path":"EuroLLM-1.7B-Instruct.Q4_K_M.gguf","size":1045157088,"lfs":{{"oid":"{oid}","size":1045157088}}}}]"#
        );
        assert_eq!(parse_tree(&tree, "EuroLLM-1.7B-Instruct.Q4_K_M.gguf"), Some(Remote { size: 1_045_157_088, oid: oid.into() }));
        assert_eq!(parse_tree(&tree, "README.md"), None, "fichier sans empreinte publiée");
        assert_eq!(parse_tree("pas du json", "x"), None);
    }
}
