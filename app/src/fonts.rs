//! Extraction des polices du chat depuis les données locales du jeu (lecture seule),
//! vers le cache local de l'application, pour que le chat miroir (S5) dessine
//! exactement la même police que WoW. Les polices restent sur le PC du joueur :
//! elles ne sont jamais redistribuées.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::casc::{self, Storage};
use crate::install;
use crate::md5::{hex, md5};
use crate::ttf::Font;

/// Police du chat par défaut (roman et cyrillique) et police d'interface.
pub const DEFAULT_FONTS: &[&str] = &[
    "Fonts\\ARIALN.TTF",
    "Fonts\\FRIZQT__.TTF",
    "Fonts\\FRIZQT___CYR.TTF",
];

/// Lettres propres à chacune des 23 langues (en plus de l'ASCII).
pub const SAMPLES: &[(&str, &str)] = &[
    ("EN", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"),
    ("FR", "àâæçéèêëîïôœùûüÿÀÂÆÇÉÈÊËÎÏÔŒÙÛÜŸ«»"),
    ("DE", "äöüßÄÖÜ„“"),
    ("ES", "áéíñóúüÁÉÍÑÓÚÜ¿¡"),
    ("IT", "àèéìíîòóùúÀÈÉÌÒÙ"),
    ("PT", "ãõáâàçéêíóôúÃÕÁÂÀÇÉÊÍÓÔÚ"),
    ("NL", "áéíóúàèëïöüÁÉËÏÖÜ"),
    ("PL", "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ"),
    ("CS", "áčďéěíňóřšťúůýžÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ"),
    ("SK", "áäčďéíĺľňóôŕšťúýžÁÄČĎÉÍĹĽŇÓÔŔŠŤÚÝŽ"),
    ("HU", "áéíóöőúüűÁÉÍÓÖŐÚÜŰ"),
    ("RO", "ăâîșțĂÂÎȘȚ"),
    ("RO-cédille", "şţŞŢ"),
    ("HR", "čćđšžČĆĐŠŽ"),
    ("SL", "čšžČŠŽ"),
    ("SV", "åäöÅÄÖ"),
    ("NO", "æøåÆØÅ"),
    ("FI", "äöåÄÖÅšž"),
    ("DA", "æøåÆØÅ"),
    ("RU", "абвгдеёжзийклмнопрстуфхцчшщъыьэюяАБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ"),
    ("UK", "абвгґдеєжзиіїйклмнопрстуфхцчшщьюяАБВГҐДЕЄЖЗИІЇЙКЛМНОПРСТУФХЦЧШЩЬЮЯ"),
    ("BG", "абвгдежзийклмнопрстуфхцчшщъьюяАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЬЮЯ"),
    ("SR", "абвгдђежзијклљмнњопрстћуфхцчџшАБВГДЂЕЖЗИЈКЛЉМНЊОПРСТЋУФХЦЧЏШ"),
    ("EL", "αβγδεζηθικλμνξοπρσςτυφχψωάέήίόύώϊϋΐΰΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩΆΈΉΊΌΎΏ"),
    ("Ponctuation", "’‘“”…–—€·"),
];

/// Dossier de cache de l'application (%LOCALAPPDATA%\ChevaliersTranslators sous Windows).
pub fn cache_root() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("ChevaliersTranslators")
}

/// Langue du texte du client : Config.wtf, sinon `.build.info`, sinon enUS.
pub fn client_locale(wow: &Path, flavor: &str, tags: &str) -> (String, &'static str) {
    let cfg = wow.join(flavor).join("WTF").join("Config.wtf");
    if let Ok(text) = fs::read_to_string(cfg) {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next().is_some_and(|w| w.eq_ignore_ascii_case("SET"))
                && it.next().is_some_and(|w| w.eq_ignore_ascii_case("textLocale"))
            {
                let v = it.next().unwrap_or("").trim_matches('"');
                if casc::locale_flag(v).is_some() {
                    return (v.to_string(), "Config.wtf");
                }
            }
        }
    }
    for part in tags.split(':').filter(|p| p.contains("text?")) {
        if let Some(w) = part
            .split_whitespace()
            .find(|w| casc::locale_flag(w).is_some())
        {
            return (w.to_string(), ".build.info");
        }
    }
    ("enUS".into(), "par défaut")
}

/// Langue du client d'une version de WoW (Config.wtf, sinon .build.info de son produit).
pub fn client_locale_of(wow: &Path, flavor: &str) -> String {
    let products = install::products_for(flavor).unwrap_or(&[]);
    let tags = install::build_info_rows(wow)
        .into_iter()
        .find(|r| r.get("product").is_some_and(|p| products.contains(&p.as_str())))
        .and_then(|r| r.get("tags").cloned())
        .unwrap_or_default();
    client_locale(wow, flavor, &tags).0
}

fn disk_path(wow: &Path, flavor: &str, game_path: &str) -> PathBuf {
    game_path
        .split(['\\', '/'])
        .filter(|p| !p.is_empty())
        .fold(wow.join(flavor), |acc, p| acc.join(p))
}

fn file_name(game_path: &str) -> String {
    game_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(game_path)
        .to_string()
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

/// Rapport de couverture : langues complètes sur une ligne, manques détaillés ensuite.
pub fn coverage_lines(font: &Font) -> Vec<String> {
    let mut ok = Vec::new();
    let mut lines = Vec::new();
    for (lang, sample) in SAMPLES {
        let miss = font.missing(sample);
        if miss.is_empty() {
            ok.push(*lang);
        } else {
            let shown: String = miss
                .iter()
                .take(12)
                .map(|c| format!("{c} "))
                .collect();
            let more = if miss.len() > 12 {
                format!("(+{})", miss.len() - 12)
            } else {
                String::new()
            };
            lines.push(format!(
                "      {lang} : manque {} caractère(s) : {}{more}",
                miss.len(),
                shown
            ));
        }
    }
    let mut out = vec![format!(
        "      complet : {}",
        if ok.is_empty() {
            "aucune langue".into()
        } else {
            ok.join(" ")
        }
    )];
    out.extend(lines);
    out
}

/// Police du jeu pour le chat miroir : depuis le cache local, sinon extraite des archives
/// du jeu (puis gardée en cache) ; les polices d'addons sont lues sur le disque.
pub fn load_game_font(wow: &Path, flavor: &str, game_path: &str) -> Result<(Vec<u8>, String), String> {
    let lower = game_path.to_ascii_lowercase();
    if lower.starts_with("interface\\") || lower.starts_with("interface/") {
        let p = disk_path(wow, flavor, game_path);
        return fs::read(&p)
            .map(|d| (d, format!("fichier {}", p.display())))
            .map_err(|e| format!("{} : {e}", p.display()));
    }
    let products = install::products_for(flavor).ok_or_else(|| format!("version inconnue : {flavor}"))?;
    let rows = install::build_info_rows(wow);
    let row = rows
        .iter()
        .filter(|r| r.get("product").is_some_and(|p| products.contains(&p.as_str())))
        .max_by_key(|r| r.get("active").map(String::as_str) != Some("0"))
        .ok_or_else(|| format!("produit {} absent de .build.info", products.join("/")))?;
    let get = |k: &str| row.get(k).cloned().unwrap_or_default();
    let dir = cache_root()
        .join("fonts")
        .join(format!("{}-{}", get("product"), get("version")));
    let fdid = game_path.strip_prefix("FDID:").and_then(|f| f.parse::<u32>().ok());
    let name = match fdid {
        Some(f) => format!("FDID_{f}.ttf"),
        None => file_name(game_path),
    };
    if let Ok(d) = fs::read(dir.join(&name)) {
        return Ok((d, format!("cache {}", dir.join(&name).display())));
    }
    let mut st = Storage::open(wow, products)?;
    let (locale, _) = client_locale(wow, flavor, &get("tags"));
    let lflag = casc::locale_flag(&locale).unwrap_or(0x2);
    let hits = match fdid {
        Some(f) => st.find_fdid(f)?,
        None => st.find_all(game_path)?,
    };
    let h = casc::pick(&hits, lflag).ok_or_else(|| format!("{game_path} absent des archives du jeu"))?;
    let data = st.read_ckey(&h.ckey)?;
    fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    fs::write(dir.join(&name), &data).map_err(|e| format!("{} : {e}", dir.display()))?;
    Ok((data, format!("extraite du jeu (FDID {}) vers {}", h.fdid, dir.display())))
}

/// Polices de secours pour les caractères absents de la police du jeu (ex. ș ț roumains).
pub fn fallback_fonts() -> Vec<(String, Vec<u8>)> {
    let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let mut paths: Vec<PathBuf> = ["ARIALN.TTF", "arial.ttf", "segoeui.ttf", "seguisym.ttf"]
        .iter()
        .map(|f| Path::new(&win).join("Fonts").join(f))
        .collect();
    paths.extend(
        [
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .iter()
        .map(PathBuf::from),
    );
    paths
        .into_iter()
        .filter_map(|p| {
            let d = fs::read(&p).ok()?;
            Some((p.display().to_string(), d))
        })
        .collect()
}

pub struct Options {
    pub wow: PathBuf,
    pub flavor: String,
    pub locale: Option<String>,
    pub files: Vec<String>,
    /// Repli : fichiers désignés par leur FileDataID.
    pub fdids: Vec<u32>,
    pub out: Option<PathBuf>,
}

pub fn run(o: &Options) -> Result<bool, String> {
    let products = install::products_for(&o.flavor).ok_or_else(|| {
        format!(
            "version inconnue : {} (connues : {})",
            o.flavor,
            install::known_flavors().join(", ")
        )
    })?;
    let t0 = Instant::now();
    let mut st = Storage::open(&o.wow, products)?;
    let opened = t0.elapsed();
    let tags = install::build_info_rows(&o.wow)
        .into_iter()
        .find(|r| r.get("product") == Some(&st.product))
        .and_then(|r| r.get("tags").cloned())
        .unwrap_or_default();
    let (locale, source) = match &o.locale {
        Some(l) => (l.clone(), "imposée"),
        None => client_locale(&o.wow, &o.flavor, &tags),
    };
    let lflag = casc::locale_flag(&locale).ok_or_else(|| {
        format!(
            "langue inconnue : {locale} (attendu : {})",
            casc::LOCALES
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let out = o.out.clone().unwrap_or_else(|| {
        cache_root()
            .join("fonts")
            .join(format!("{}-{}", st.product, st.version))
    });
    fs::create_dir_all(&out).map_err(|e| format!("{} : {e}", out.display()))?;

    crate::say!(
        "Installation : {} - {} (produit {} {}, build {})",
        o.wow.display(),
        o.flavor,
        st.product,
        st.version,
        &st.build_key[..8.min(st.build_key.len())]
    );
    crate::say!("Langue du client : {locale} ({source})");
    crate::say!(
        "Stockage : racine {}, {} entrées dont {} nommées, {} blocs ; ouvert en {:.1} s",
        st.root_format.label(),
        thousands(st.root_stats.entries),
        thousands(st.root_stats.named),
        st.root_stats.blocks,
        opened.as_secs_f64()
    );
    crate::say!("Cache local : {}", out.display());

    let mut files: Vec<String> = if o.files.is_empty() && o.fdids.is_empty() {
        DEFAULT_FONTS.iter().map(|s| s.to_string()).collect()
    } else {
        o.files.clone()
    };
    files.extend(o.fdids.iter().map(|f| format!("FDID {f}")));
    let mut manifest = String::from("# chemin du jeu\tfichier\tFDID\tMD5\toctets\tpolice\n");
    let mut all_ok = true;
    for gp in &files {
        crate::say!("");
        let from_disk = gp.to_ascii_lowercase().starts_with("interface\\")
            || gp.to_ascii_lowercase().starts_with("interface/");
        let got: Result<(Vec<u8>, String, String), String> = if from_disk {
            // Police d'un addon (ElvUI, Prat...) : fichier ordinaire du dossier du jeu.
            let p = disk_path(&o.wow, &o.flavor, gp);
            fs::read(&p)
                .map(|d| (d, "-".into(), format!("fichier {}", p.display())))
                .map_err(|e| format!("{} : {e}", p.display()))
        } else {
            let fdid = gp.strip_prefix("FDID ").and_then(|f| f.parse::<u32>().ok());
            let found = match fdid {
                Some(f) => st.find_fdid(f),
                None => st.find_all(gp),
            };
            match found {
                Err(e) => Err(e),
                Ok(hits) if hits.is_empty() => Err(if fdid.is_some() {
                    "FileDataID absent de cette version du jeu".into()
                } else {
                    "nom absent du stockage (chemin inconnu de cette version, ou nom retiré \
                     par Blizzard : réessayer avec --fdid <FileDataID>)"
                        .into()
                }),
                Ok(hits) => {
                    let h = casc::pick(&hits, lflag).unwrap();
                    let variants = if hits.len() > 1 {
                        format!(", {} variantes", hits.len())
                    } else {
                        String::new()
                    };
                    st.read_ckey(&h.ckey).map(|d| {
                        (
                            d,
                            h.fdid.to_string(),
                            format!(
                                "FDID {}, langue {}{variants}, MD5 vérifié",
                                h.fdid,
                                casc::locale_names(h.locale)
                            ),
                        )
                    })
                }
            }
        };
        let (data, fdid, how) = match got {
            Ok(v) => v,
            Err(e) => {
                crate::say!("{gp} : ÉCHEC - {e}");
                all_ok = false;
                continue;
            }
        };
        crate::say!("{gp} : {} octets, {how}", thousands(data.len()));
        let name = match gp.strip_prefix("FDID ") {
            Some(f) => format!("FDID_{f}.ttf"),
            None => file_name(gp),
        };
        let dest = out.join(&name);
        if let Err(e) = fs::write(&dest, &data) {
            crate::say!("   écriture de {} impossible : {e}", dest.display());
            all_ok = false;
            continue;
        }
        let family = match Font::parse(&data) {
            Ok(f) => {
                crate::say!(
                    "   police « {} » ({}), {} caractères -> {}",
                    f.family,
                    f.full,
                    thousands(f.char_count() as usize),
                    dest.display()
                );
                for l in coverage_lines(&f) {
                    crate::say!("{l}");
                }
                f.family
            }
            Err(e) => {
                crate::say!("   -> {} (illisible comme police : {e})", dest.display());
                all_ok = false;
                String::new()
            }
        };
        manifest.push_str(&format!(
            "{gp}\t{name}\t{fdid}\t{}\t{}\t{family}\n",
            hex(&md5(&data)),
            data.len()
        ));
    }
    if let Some(o) = st.hash_order {
        crate::say!("");
        crate::say!(
            "Hash des chemins : ordre {} (information pour la suite du projet)",
            if o == 0 { "c:b" } else { "b:c" }
        );
    }
    fs::write(out.join("manifest.tsv"), manifest).map_err(|e| e.to_string())?;
    fs::write(
        out.join("LISEZMOI.txt"),
        "Polices copiées depuis votre installation de World of Warcraft par Chevaliers Translators,\r\n\
         uniquement pour afficher le chat miroir sur ce PC. Elles appartiennent à leurs auteurs\r\n\
         et à Blizzard Entertainment : ne pas les redistribuer. Ce dossier peut être supprimé\r\n\
         sans risque ; il sera recréé à partir des fichiers du jeu.\r\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::casc::fixture;

    #[test]
    fn locale_detection_order() {
        let dir = std::env::temp_dir().join(format!("ct-loc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let tags = "Windows x86_64 EU? deDE speech?:Windows x86_64 EU? esES text?";
        assert_eq!(client_locale(&dir, "_forever_", tags).0, "esES");
        assert_eq!(client_locale(&dir, "_forever_", "").0, "enUS");
        let wtf = dir.join("_forever_").join("WTF");
        fs::create_dir_all(&wtf).unwrap();
        fs::write(wtf.join("Config.wtf"), "SET textLocale \"ruRU\"\r\n").unwrap();
        assert_eq!(
            client_locale(&dir, "_forever_", tags),
            ("ruRU".into(), "Config.wtf")
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn extracts_from_fixture_and_disk() {
        let dir = std::env::temp_dir().join(format!("ct-fonts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // La police française du stockage synthétique est remplacée par une vraie police.
        let mut entries = fixture::default_entries();
        let font = crate::ttf::tests::tiny_font(true);
        entries[1].content = font.clone();
        fixture::build(&dir, &entries).unwrap();
        let addon = dir.join("_forever_").join("Interface").join("AddOns").join("FakeUI");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("Font.ttf"), &font).unwrap();

        let out = dir.join("cache");
        let ok = run(&Options {
            wow: dir.clone(),
            flavor: "_forever_".into(),
            locale: None,
            files: vec![
                "Fonts\\ARIALN.TTF".into(),
                "Interface\\AddOns\\FakeUI\\Font.ttf".into(),
            ],
            fdids: vec![1000],
            out: Some(out.clone()),
        })
        .unwrap();
        assert!(ok);
        assert_eq!(fs::read(out.join("ARIALN.TTF")).unwrap(), font);
        assert_eq!(fs::read(out.join("Font.ttf")).unwrap(), font);
        assert_eq!(fs::read(out.join("FDID_1000.ttf")).unwrap(), font);
        let manifest = fs::read_to_string(out.join("manifest.tsv")).unwrap();
        assert!(manifest.contains("Fonts\\ARIALN.TTF\tARIALN.TTF\t1000\t"));
        assert!(manifest.contains("CT Test"));

        // Fichier absent et produit inconnu : signalés, sans arrêt brutal.
        let missing = run(&Options {
            wow: dir.clone(),
            flavor: "_forever_".into(),
            locale: Some("deDE".into()),
            files: vec!["Fonts\\ABSENTE.TTF".into()],
            fdids: vec![],
            out: Some(out),
        })
        .unwrap();
        assert!(!missing);
        assert!(run(&Options {
            wow: dir.clone(),
            flavor: "_inconnu_".into(),
            locale: None,
            files: vec![],
            fdids: vec![],
            out: None,
        })
        .is_err());
        fs::remove_dir_all(dir).ok();
    }
}
