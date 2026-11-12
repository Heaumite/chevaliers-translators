//! Installation de l'addon dans toutes les versions du jeu, avec le numéro d'interface lu dans
//! `.build.info` (version 12.1.5 -> interface 120105), et du petit addon de présence que
//! l'application réécrit pendant qu'elle tourne.

use std::fs;
use std::path::{Path, PathBuf};

const ENCODER: &str = include_str!("../../addon/ChevaliersTranslators/Encoder.lua");
const BANNER: &str = include_str!("../../addon/ChevaliersTranslators/Banner.lua");
const CORE: &str = include_str!("../../addon/ChevaliersTranslators/Core.lua");
const MIRROR: &str = include_str!("../../addon/ChevaliersTranslators/Mirror.lua");
/// Logo de la liste des addons (TGA 64x64, ligne `## IconTexture` du .toc).
const LOGO: &[u8] = include_bytes!("../../addon/ChevaliersTranslators/Media/Logo.tga");
/// Bannière du guide d'installation (TGA 512x128).
const BANNIERE: &[u8] = include_bytes!("../../addon/ChevaliersTranslators/Media/Banniere.tga");
const ICON_TEXTURE: &str = "## IconTexture: Interface\\AddOns\\ChevaliersTranslators\\Media\\Logo\n";
/// Licence et dépôt du code source (0.14.3, GPL-3.0), lignes du .toc.
const TOC_SOURCE: &str = "## X-License: GPL-3.0-or-later\n## X-Website: https://github.com/Heaumite/chevaliers-translators\n";

/// Fichiers de l'addon, dans l'ordre de chargement du .toc.
const FILES: [&str; 4] = ["Encoder.lua", "Banner.lua", "Core.lua", "Mirror.lua"];

/// Dossier de l'addon dans `Interface\AddOns`.
pub const ADDON: &str = "ChevaliersTranslators";
/// Dossiers de la phase 0 (addon de test, présence, emplacements S6), retirés à l'installation.
const LEGACY: &[&str] = &["CT_Spike", "CT_Spike_Live"];

/// Dossiers de jeu connus et produits Battle.net correspondants.
const FLAVORS: &[(&str, &[&str])] = &[
    ("_retail_", &["wow"]),
    (
        "_forever_",
        &[
            "wow_forever",
            "wowforever",
            "wowf",
            "wow_camelot",
            "camelot",
        ],
    ),
    ("_classic_era_", &["wow_classic_era"]),
    ("_anniversary_", &["wow_anniversary"]),
    ("_classic_", &["wow_classic"]),
    ("_ptr_", &["wowt"]),
    ("_xptr_", &["wowxptr"]),
    ("_beta_", &["wow_beta"]),
    ("_classic_ptr_", &["wow_classic_ptr"]),
    ("_classic_era_ptr_", &["wow_classic_era_ptr"]),
];

/// Lignes de `.build.info`, colonnes nommées en minuscules ("product", "build key"...).
pub fn build_info_rows(root: &Path) -> Vec<std::collections::HashMap<String, String>> {
    let Ok(text) = fs::read_to_string(root.join(".build.info")) else {
        return Vec::new();
    };
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<String> = header
        .split('|')
        .map(|c| c.split('!').next().unwrap_or("").trim().to_lowercase())
        .collect();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            cols.iter()
                .cloned()
                .zip(l.split('|').map(|v| v.trim().to_string()))
                .collect()
        })
        .collect()
}

/// Lit `.build.info` : couples (produit, version) des installations actives.
pub fn read_build_info(root: &Path) -> Vec<(String, String)> {
    build_info_rows(root)
        .into_iter()
        .filter(|r| r.get("active").map(String::as_str) != Some("0"))
        .filter_map(|r| {
            Some((
                r.get("product")?.to_lowercase(),
                r.get("version")?.clone(),
            ))
        })
        .collect()
}

/// Produits Battle.net d'un dossier de jeu connu (`_forever_` -> wowf...).
pub fn products_for(folder: &str) -> Option<&'static [&'static str]> {
    FLAVORS.iter().find(|(f, _)| *f == folder).map(|(_, p)| *p)
}

pub fn known_flavors() -> Vec<&'static str> {
    FLAVORS.iter().map(|(f, _)| *f).collect()
}

/// Versions du jeu présentes dans un dossier de WoW (`_retail_`, `_forever_`...).
pub fn flavor_dirs(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.len() > 2 && n.starts_with('_') && n.ends_with('_'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Version de l'addon installée dans une version du jeu (ligne « ## Version » de son .toc).
pub fn addon_version(root: &Path, flavor: &str) -> Option<String> {
    let toc = fs::read_to_string(addons_dir(root, flavor).join(ADDON).join(format!("{ADDON}.toc"))).ok()?;
    toc.lines()
        .find_map(|l| l.strip_prefix("## Version:"))
        .map(|v| v.trim().to_string())
        .or(Some("inconnue".into()))
}

/// État de l'addon dans chaque version du jeu.
pub fn status(root: &Path) -> Vec<(String, Option<String>)> {
    flavor_dirs(root).into_iter().map(|f| {
        let v = addon_version(root, &f);
        (f, v)
    }).collect()
}

pub fn status_report(root: &Path) -> String {
    let me = env!("CARGO_PKG_VERSION");
    let mut s = format!("Addon dans {} :", root.display());
    let st = status(root);
    if st.is_empty() {
        s.push_str(" aucune version du jeu trouvée");
    }
    for (f, v) in st {
        s.push_str(&format!(
            "\n  {f} : {}",
            match v {
                None => "absent".to_string(),
                Some(v) if v == me => format!("{v}, à jour"),
                Some(v) => format!("{v}, ancienne version (l'application est en {me}) : relancer l'installation"),
            }
        ));
    }
    s
}

/// Dossier de WoW à partir d'un chemin donné par Windows (souvent celui de `_retail_`).
pub fn wow_root_of(p: &Path) -> PathBuf {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.len() > 2 && name.starts_with('_') && name.ends_with('_') {
        p.parent().map(Path::to_path_buf).unwrap_or_else(|| p.to_path_buf())
    } else {
        p.to_path_buf()
    }
}

/// Un dossier de WoW : `.build.info`, ou au moins une version du jeu.
pub fn is_wow_root(p: &Path) -> bool {
    p.join(".build.info").is_file() || !flavor_dirs(p).is_empty() && p.join("Data").is_dir()
}

/// Dossiers de WoW de ce PC : ceux qu'indique Windows (registre), puis les emplacements
/// courants sur chaque disque. Sans doublon, dans cet ordre.
pub fn find_wow() -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = registry_paths().iter().map(|p| wow_root_of(p)).collect();
    for d in 'C'..='Z' {
        for rel in [
            "World of Warcraft",
            "Program Files (x86)\\World of Warcraft",
            "Program Files\\World of Warcraft",
            "Games\\World of Warcraft",
            "Jeux\\World of Warcraft",
            "Blizzard\\World of Warcraft",
            "Battle.net\\World of Warcraft",
        ] {
            cands.push(PathBuf::from(format!("{d}:\\{rel}")));
        }
    }
    find_wow_in(&cands)
}

pub fn find_wow_in(cands: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for c in cands {
        let key = c.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        if is_wow_root(c) && !out.iter().any(|o| o.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase() == key) {
            out.push(c.clone());
        }
    }
    out
}

/// Chemins de WoW notés par le programme d'installation de Blizzard (Windows).
fn registry_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let keys = [
            ("HKLM\\SOFTWARE\\WOW6432Node\\Blizzard Entertainment\\World of Warcraft", "InstallPath"),
            ("HKLM\\SOFTWARE\\Blizzard Entertainment\\World of Warcraft", "InstallPath"),
            ("HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\World of Warcraft", "InstallLocation"),
        ];
        let mut out = Vec::new();
        for (k, v) in keys {
            let Ok(o) = crate::quiet(&mut std::process::Command::new("reg")).args(["query", k, "/v", v]).output() else { continue };
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some(i) = line.find("REG_SZ") {
                    let p = line[i + 6..].trim().trim_end_matches(['\\', '/']);
                    if !p.is_empty() {
                        out.push(PathBuf::from(p));
                    }
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// "12.1.5.66789" -> 120105
pub fn interface_from_version(v: &str) -> Option<u32> {
    let mut it = v.split('.').map(|p| p.parse::<u32>().ok());
    let (a, b, c) = (it.next()??, it.next()??, it.next()??);
    Some(a * 10000 + b * 100 + c)
}

fn interface_for(folder: &str, info: &[(String, String)]) -> Option<(u32, String)> {
    let stem = folder.trim_matches('_');
    let found = match FLAVORS.iter().find(|(f, _)| *f == folder) {
        // Dossier connu : uniquement ses produits (évite de confondre _classic_ et _classic_era_).
        Some((_, products)) => info.iter().find(|(p, _)| products.contains(&p.as_str())),
        // Dossier inconnu (nouvelle version) : produit dont le nom contient celui du dossier.
        None => info.iter().find(|(p, _)| p.contains(stem)),
    };
    found.and_then(|(p, v)| interface_from_version(v).map(|i| (i, format!("{p} {v}"))))
}

fn toc(iface: u32, title: &str, extra: &str, files: &[&str]) -> String {
    let mut s = format!("## Interface: {iface}\n## Title: {title}\n{extra}");
    s.push('\n');
    for f in files {
        s.push_str(f);
        s.push('\n');
    }
    s
}

/// Désinstallation (0.14.3) : ce qui est à retirer. L'addon et sa présence dans toutes les versions
/// du jeu de chaque dossier de WoW ; avec `data`, aussi les réglages de l'addon (SavedVariables de
/// chaque compte) et les données de l'application (`data_dir` : modèles, moteur, journaux).
pub fn uninstall_targets(roots: &[PathBuf], data: bool, data_dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for root in roots {
        for folder in flavor_dirs(root) {
            let addons = addons_dir(root, &folder);
            for name in [ADDON, LIVE_ADDON] {
                let p = addons.join(name);
                if p.exists() {
                    v.push(p);
                }
            }
            if !data {
                continue;
            }
            let Ok(accounts) = fs::read_dir(root.join(&folder).join("WTF").join("Account")) else { continue };
            for a in accounts.flatten() {
                for f in [format!("{ADDON}.lua"), format!("{ADDON}.lua.bak")] {
                    let p = a.path().join("SavedVariables").join(f);
                    if p.is_file() {
                        v.push(p);
                    }
                }
            }
        }
    }
    if data && data_dir.exists() {
        v.push(data_dir.to_path_buf());
    }
    v
}

/// Retire ce qui peut l'être tout de suite ; renvoie ce qui reste (fichiers en cours d'usage :
/// l'exécutable lancé depuis le dossier de l'addon, le journal ouvert), à retirer après l'arrêt.
pub fn remove_now(targets: &[PathBuf]) -> Vec<PathBuf> {
    targets
        .iter()
        .filter(|p| {
            let r = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
            r.is_err() && p.exists()
        })
        .cloned()
        .collect()
}

/// Taille d'un dossier (octets), sous-dossiers compris.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(p) else { return fs::metadata(p).map_or(0, |m| m.len()) };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

/// Chaîne Lua sûre : guillemets, barres obliques et caractères de contrôle échappés.
pub fn lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\{:03}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Taille fixe du fichier de présence : il garde la même longueur à chaque réécriture, écrite
/// par-dessus l'ancienne sans jamais vider le fichier (le jeu ne lit jamais un fichier vide).
pub const LIVE_LEN: usize = 256;

/// Écran de WoW pour l'addon (0.14), dans le fichier de présence (`e`) : 0 écran de PC, 1
/// télévision (dispositif télévision : le chat du jeu reste dessous, le chat traduit le recouvre).
static SCREEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_screen_tv(tv: bool) {
    SCREEN.store(tv as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn screen_tv() -> bool {
    SCREEN.load(std::sync::atomic::Ordering::Relaxed) == 1
}

/// Fichier de présence de l'application (addon ChevaliersTranslators_Live, chargé à chaque
/// connexion et /reload) : l'heure `t`, réécrite toutes les 2 s par l'application, 0 quand elle
/// est quittée ; `lu`, l'heure à laquelle le jeu exécute le fichier. L'addon compare les deux :
/// la présence est jugée au moment de la lecture, quelle que soit la durée de l'écran de
/// chargement qui suit. Récente, la traduction du chat démarre seule.
pub fn live_lua(t: u64, version: &str) -> String {
    let mut s = format!(
        "-- Réécrit par l'application toutes les 2 s : la traduction démarre seule à la connexion.\nCHEVALIERS_LIVE = {{ t = {t:>12}, v = {}, e = {}, lu = time and time() }}\n",
        lua_string(version),
        SCREEN.load(std::sync::atomic::Ordering::Relaxed)
    );
    if s.len() + 3 < LIVE_LEN {
        s.push_str("--");
        while s.len() < LIVE_LEN - 1 {
            s.push('-');
        }
        s.push('\n');
    }
    s
}

/// Réécrit le fichier de présence dans le dossier AddOns d'une version du jeu.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn write_live(addons: &Path) -> Result<(), String> {
    write_presence(addons, crate::catalog::now_secs())
}

/// Présence `t` (0 : application quittée) écrite par-dessus l'ancienne, même longueur.
fn write_presence(addons: &Path, t: u64) -> Result<(), String> {
    use std::io::Write;
    let dir = addons.join(LIVE_ADDON);
    if !dir.is_dir() {
        return Err(format!("{} absent : relance l'application (elle installe l'addon), puis redémarre WoW", dir.display()));
    }
    let body = live_lua(t, env!("CARGO_PKG_VERSION"));
    let write = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(false).open(dir.join("Live.lua"))?;
        f.write_all(body.as_bytes())?;
        f.set_len(body.len() as u64)
    };
    write().map_err(|e| format!("{} : {e}", dir.display()))
}

/// Présence écrite dans toutes les versions du jeu d'un dossier WoW : `running` faux à la
/// fermeture de l'application (un /reload rend alors le chat officiel tout de suite).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn presence(root: &Path, running: bool) -> usize {
    let t = if running { crate::catalog::now_secs() } else { 0 };
    flavor_dirs(root)
        .iter()
        .map(|f| addons_dir(root, f))
        .filter(|d| d.join(LIVE_ADDON).is_dir())
        .filter(|d| write_presence(d, t).is_ok())
        .count()
}

pub const LIVE_ADDON: &str = "ChevaliersTranslators_Live";

/// Réécrit le fichier de présence dans toutes les versions du jeu installées à côté de `addons`
/// (dossier AddOns d'une version) : l'addon le voit quelle que soit la version lancée.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn write_live_everywhere(addons: &Path) -> Result<usize, String> {
    let root = addons.parent().and_then(Path::parent).and_then(Path::parent);
    let mut dirs: Vec<PathBuf> = match root {
        Some(r) => flavor_dirs(r).iter().map(|f| addons_dir(r, f)).filter(|d| d.join(LIVE_ADDON).is_dir()).collect(),
        None => Vec::new(),
    };
    if !dirs.iter().any(|d| d == addons) {
        dirs.push(addons.to_path_buf());
    }
    let mut ok = 0;
    let mut last_err = None;
    for d in dirs {
        match write_live(&d) {
            Ok(()) => ok += 1,
            Err(e) => last_err = Some(e),
        }
    }
    match (ok, last_err) {
        (0, Some(e)) => Err(e),
        _ => Ok(ok),
    }
}

pub fn addons_dir(root: &Path, folder: &str) -> PathBuf {
    root.join(folder).join("Interface").join("AddOns")
}

/// Retire les dossiers de la phase 0 (CT_Spike, sa présence, ses emplacements CT_SlotNNN).
fn remove_legacy(addons: &Path) -> usize {
    let mut names: Vec<String> = LEGACY.iter().map(|s| s.to_string()).collect();
    names.extend((1..=99).map(|i| format!("CT_Slot{i:03}")));
    names.into_iter().filter(|n| addons.join(n).is_dir() && fs::remove_dir_all(addons.join(n)).is_ok()).count()
}

/// Réglages de la phase 0 repris : `WTF\Account\<compte>\SavedVariables\CT_Spike.lua` copié sous
/// le nom du nouvel addon (variable renommée), si celui-ci n'a pas encore de réglages. Le jeu ne
/// relit les réglages qu'à son démarrage, d'où la copie faite à l'installation.
fn migrate_settings(flavor_dir: &Path) -> usize {
    let Ok(accounts) = fs::read_dir(flavor_dir.join("WTF").join("Account")) else { return 0 };
    let mut n = 0;
    for acc in accounts.flatten() {
        let sv = acc.path().join("SavedVariables");
        let (old, new) = (sv.join("CT_Spike.lua"), sv.join(format!("{ADDON}.lua")));
        if new.exists() {
            continue;
        }
        if let Ok(body) = fs::read_to_string(&old) {
            if fs::write(&new, body.replace("CT_SpikeDB", "ChevaliersTranslatorsDB")).is_ok() {
                n += 1;
            }
        }
    }
    n
}

pub fn install(root: &Path, only: Option<&str>, iface_override: Option<u32>) -> Result<(), String> {
    if !root.is_dir() {
        return Err(format!("dossier introuvable : {}", root.display()));
    }
    let info = read_build_info(root);
    let mut folders = flavor_dirs(root);
    if let Some(f) = only {
        folders.retain(|n| n == f);
        if folders.is_empty() {
            return Err(format!("version {f} absente de {}", root.display()));
        }
    }
    if folders.is_empty() {
        return Err(
            "aucune version de WoW trouvée (dossiers _retail_, _forever_, _classic_era_...)".into(),
        );
    }

    let mut done = 0;
    for folder in &folders {
        let (iface, source) = match (iface_override, interface_for(folder, &info)) {
            (Some(i), _) => (i, "valeur imposée".to_string()),
            (None, Some((i, s))) => (i, format!(".build.info : {s}")),
            (None, None) => {
                crate::say!("{folder} : version introuvable dans .build.info, ignoré (préciser --flavor {folder} --interface <n>)");
                continue;
            }
        };
        let addons = addons_dir(root, folder);
        let removed = remove_legacy(&addons);
        migrate_settings(&root.join(folder));
        let main = addons.join(ADDON);
        fs::create_dir_all(&main).map_err(|e| format!("{} : {e}", main.display()))?;
        let files = [
            ("Encoder.lua", ENCODER),
            ("Banner.lua", BANNER),
            ("Core.lua", CORE),
            ("Mirror.lua", MIRROR),
        ];
        for (name, body) in files {
            fs::write(main.join(name), body).map_err(|e| format!("{name} : {e}"))?;
        }
        let media = main.join("Media");
        fs::create_dir_all(&media).map_err(|e| format!("{} : {e}", media.display()))?;
        fs::write(media.join("Logo.tga"), LOGO).map_err(|e| format!("Logo.tga : {e}"))?;
        fs::write(media.join("Banniere.tga"), BANNIERE).map_err(|e| format!("Banniere.tga : {e}"))?;
        // Raccourci de la phase 0, chargé d'office par le jeu s'il reste là : plus utilisé.
        let _ = fs::remove_file(main.join("Bindings.xml"));
        let main_toc = toc(
            iface,
            "Chevaliers Translators",
            &format!(
                "## Notes: Traduction du chat entre les langues européennes, sur ton PC. /chevaliers\n## Author: Chevaliers Translators\n## Version: {}\n## SavedVariables: ChevaliersTranslatorsDB\n{ICON_TEXTURE}{TOC_SOURCE}",
                env!("CARGO_PKG_VERSION")
            ),
            &FILES,
        );
        fs::write(main.join(format!("{ADDON}.toc")), main_toc).map_err(|e| e.to_string())?;

        // Présence de l'application : petit addon chargé à chaque connexion, réécrit par elle.
        let live = addons.join(LIVE_ADDON);
        fs::create_dir_all(&live).map_err(|e| format!("{} : {e}", live.display()))?;
        fs::write(
            live.join(format!("{LIVE_ADDON}.toc")),
            toc(
                iface,
                "Chevaliers Translators - présence de l'application",
                &format!("## Notes: Réécrit par l'application pendant qu'elle tourne\n## Version: {}\n## Group: {ADDON}\n", env!("CARGO_PKG_VERSION")),
                &["Live.lua"],
            ),
        )
        .map_err(|e| e.to_string())?;
        fs::write(live.join("Live.lua"), live_lua(0, env!("CARGO_PKG_VERSION"))).map_err(|e| e.to_string())?;
        crate::say!(
            "{folder} : interface {iface} ({source}) -> {}{}",
            main.display(),
            if removed > 0 { format!(" ; {removed} dossier(s) de la phase 0 retiré(s)") } else { String::new() }
        );
        // Pour l'estimation : réglages graphiques de cette version (WTF\Config.wtf).
        crate::plan::remember_wow(&root.join(folder));
        done += 1;
    }
    if done == 0 {
        return Err("rien n'a été installé".into());
    }
    crate::say!("{}", status_report(root));
    crate::say!("Relance WoW s'il était ouvert : la liste des addons n'est lue qu'au démarrage.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(interface_from_version("12.1.5.66789"), Some(120105));
        assert_eq!(interface_from_version("1.60.1.70790"), Some(16001));
        assert_eq!(interface_from_version("1.15.9.12345"), Some(11509));
        assert_eq!(interface_from_version("2.5.6.1"), Some(20506));
        assert_eq!(interface_from_version("x"), None);
    }

    #[test]
    fn build_info_parsing() {
        let dir = std::env::temp_dir().join(format!("ct-bi-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".build.info"),
            "Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|Version!STRING:0|Product!STRING:0\n\
             eu|1|abc|12.1.5.66789|wow\n\
             eu|1|def|1.60.1.70790|wow_forever\n\
             eu|0|old|1.15.8.1|wow_classic_era\n\
             eu|1|ghi|1.15.9.2|wow_classic_era\n",
        )
        .unwrap();
        let info = read_build_info(&dir);
        assert_eq!(interface_for("_retail_", &info).unwrap().0, 120105);
        assert_eq!(interface_for("_forever_", &info).unwrap().0, 16001);
        assert_eq!(interface_for("_classic_era_", &info).unwrap().0, 11509);
        assert!(interface_for("_anniversary_", &info).is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn wow_is_found_and_the_addon_state_read() {
        let base = std::env::temp_dir().join(format!("ct-find-{}", std::process::id()));
        let wow = base.join("World of Warcraft");
        fs::create_dir_all(wow.join("_forever_")).unwrap();
        fs::create_dir_all(wow.join("_retail_")).unwrap();
        fs::write(wow.join(".build.info"), "Branch!STRING:0|Active!DEC:1|Version!STRING:0|Product!STRING:0\neu|1|1.60.1.70790|wowf\neu|1|12.1.5.66789|wow\n").unwrap();
        // Chemin donné par le registre : celui de _retail_.
        assert_eq!(wow_root_of(&wow.join("_retail_")), wow);
        let found = find_wow_in(&[base.join("absent"), wow.join("_retail_").parent().unwrap().to_path_buf(), wow.clone()]);
        assert_eq!(found, vec![wow.clone()]);
        // Dossiers de la phase 0 : retirés à l'installation.
        for old in ["CT_Spike", "CT_Spike_Live", "CT_Slot003"] {
            fs::create_dir_all(addons_dir(&wow, "_forever_").join(old)).unwrap();
        }
        // Réglages de la phase 0 : repris sous le nouveau nom.
        let sv = wow.join("_forever_").join("WTF").join("Account").join("BRICE").join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        fs::write(sv.join("CT_Spike.lua"), "\nCT_SpikeDB = {\n[\"tradOff\"] = true,\n}\n").unwrap();
        // Première installation : toutes les versions présentes.
        install(&wow, None, None).unwrap();
        let st = status(&wow);
        assert_eq!(st.len(), 2);
        assert!(st.iter().all(|(_, v)| v.as_deref() == Some(env!("CARGO_PKG_VERSION"))), "{st:?}");
        assert!(addons_dir(&wow, "_forever_").join(ADDON).join("Core.lua").is_file());
        for old in ["CT_Spike", "CT_Spike_Live", "CT_Slot003"] {
            assert!(!addons_dir(&wow, "_forever_").join(old).exists(), "{old}");
        }
        let moved = fs::read_to_string(sv.join(format!("{ADDON}.lua"))).unwrap();
        assert!(moved.contains("ChevaliersTranslatorsDB = {") && !moved.contains("CT_SpikeDB"), "{moved}");
        // Réglages déjà présents : jamais écrasés.
        fs::write(sv.join(format!("{ADDON}.lua")), "ChevaliersTranslatorsDB = {}\n").unwrap();
        assert_eq!(migrate_settings(&wow.join("_forever_")), 0);
        // Présence de l'application : longueur fixe, réécrite par l'application.
        let live = addons_dir(&wow, "_forever_").join(LIVE_ADDON).join("Live.lua");
        assert_eq!(fs::read(&live).unwrap().len(), LIVE_LEN);
        write_live(&addons_dir(&wow, "_forever_")).unwrap();
        // Lancée pour une version, l'application se signale dans toutes.
        assert_eq!(write_live_everywhere(&addons_dir(&wow, "_forever_")).unwrap(), 2);
        assert_eq!(fs::read(addons_dir(&wow, "_retail_").join(LIVE_ADDON).join("Live.lua")).unwrap().len(), LIVE_LEN);
        let text = fs::read_to_string(&live).unwrap();
        assert_eq!(text.len(), LIVE_LEN);
        assert!(text.contains(&format!("v = \"{}\", e = 0, lu = time and time() }}", env!("CARGO_PKG_VERSION"))), "{text}");
        // Télévision (0.14) : `e = 1`, même longueur.
        set_screen_tv(true);
        let tv = live_lua(1_700_000_000, env!("CARGO_PKG_VERSION"));
        set_screen_tv(false);
        assert!(tv.contains(", e = 1, ") && tv.len() == LIVE_LEN, "{tv}");
        // Application quittée : t = 0 partout, même longueur.
        assert_eq!(presence(&wow, false), 2);
        let text = fs::read_to_string(&live).unwrap();
        assert!(text.contains("t =            0,") && text.len() == LIVE_LEN, "{text}");
        // Ancienne version d'un côté : signalée.
        let toc = addons_dir(&wow, "_retail_").join(ADDON).join(format!("{ADDON}.toc"));
        fs::write(&toc, "## Interface: 120105\n## Version: 0.8.7\n").unwrap();
        assert!(status_report(&wow).contains("0.8.7, ancienne version"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn uninstall_removes_the_addon_and_on_request_the_data() {
        let base = std::env::temp_dir().join(format!("ct-desinstall-{}", std::process::id()));
        let wow = base.join("World of Warcraft");
        for f in ["_retail_", "_forever_"] {
            fs::create_dir_all(wow.join(f).join("Interface").join("AddOns").join("DBM-Core")).unwrap();
        }
        fs::write(wow.join(".build.info"), "").unwrap();
        install(&wow, None, Some(120105)).unwrap();
        let sv = wow.join("_retail_").join("WTF").join("Account").join("BRICE").join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        fs::write(sv.join(format!("{ADDON}.lua")), "ChevaliersTranslatorsDB = {}\n").unwrap();
        fs::write(sv.join("DBM-Core.lua"), "DBM = {}\n").unwrap();
        let data = base.join("donnees");
        fs::create_dir_all(data.join("modeles")).unwrap();
        fs::write(data.join("modeles").join("m.gguf"), vec![0u8; 1000]).unwrap();
        assert_eq!(dir_size(&data), 1000);
        let roots = vec![wow.clone()];
        // Addon seul : addon et présence, dans chaque version ; réglages et données gardés.
        let t = uninstall_targets(&roots, false, &data);
        assert_eq!(t.len(), 4, "{t:?}");
        // Avec les données : réglages de l'addon et dossier de l'application en plus.
        let all = uninstall_targets(&roots, true, &data);
        assert_eq!(all.len(), 6, "{all:?}");
        assert!(remove_now(&all).is_empty());
        for f in ["_retail_", "_forever_"] {
            let addons = addons_dir(&wow, f);
            assert!(!addons.join(ADDON).exists() && !addons.join(LIVE_ADDON).exists());
            assert!(addons.join("DBM-Core").is_dir(), "les autres addons restent");
        }
        assert!(!sv.join(format!("{ADDON}.lua")).exists() && sv.join("DBM-Core.lua").is_file());
        assert!(!data.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn source_toc_matches_the_app() {
        // .toc du dépôt (paquet de distribution) : même version et mêmes fichiers que l'installation.
        let toc = include_str!("../../addon/ChevaliersTranslators/ChevaliersTranslators.toc");
        assert!(toc.contains(&format!("## Version: {}\n", env!("CARGO_PKG_VERSION"))), "version du .toc à mettre à jour : {toc}");
        assert!(toc.contains("## SavedVariables: ChevaliersTranslatorsDB\n"), "{toc}");
        assert!(toc.contains(ICON_TEXTURE) && toc.contains(TOC_SOURCE), "{toc}");
        let files: Vec<&str> = toc.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert_eq!(files, FILES);
        // Version relue à chaque /reload, sans redémarrage du jeu.
        assert!(CORE.contains(&format!("CT.VERSION = \"{}\"\n", env!("CARGO_PKG_VERSION"))), "CT.VERSION de Core.lua");
    }

    #[test]
    fn lua_escaping() {
        assert_eq!(lua_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(lua_string("Ça ✓"), "\"Ça ✓\"");
    }
}
