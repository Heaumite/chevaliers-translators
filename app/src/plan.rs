//! Estimation sans test (S4c) : à partir du matériel du joueur et des réglages de WoW, quel
//! modèle proposer dès l'installation, où le faire tourner (carte graphique ou processeur),
//! lesquels télécharger, et dans quel ordre se replier en jeu si le meilleur échoue.
//! La règle complète, avec ses mesures : REGLE_MODELES.md.
//!
//! Les prévisions partent des mesures du PC de référence (Radeon RX 6950 XT, 576 Go/s ;
//! WoW Forever en qualité maximale, 3840x1600, non plafonné : la carte est occupée par le jeu)
//! et les transposent à la carte du joueur :
//!   - charge imposée au jeu ∝ débit de modèle / bande passante mémoire de la carte ;
//!   - temps de traduction sur la carte ∝ 1 / bande passante, plus court si WoW laisse du
//!     temps libre à la carte (plafond d'images, écran plus petit, qualité plus basse) ;
//!   - temps de traduction sur le processeur ∝ 1 / √(fils de calcul).
//!
//! La baisse d'images par seconde d'un modèle est proportionnelle au nombre de messages
//! traduits par minute. Le critère est donc une **capacité** : combien de messages par minute
//! ce modèle traduit sur ce PC en gardant la baisse sous 5 %. Il en faut au moins 12 (un
//! message à traduire toutes les 5 s, chat très animé) ; au-delà, la traduction espace les
//! messages (`Candidate::min_gap`) au lieu de ralentir le jeu.
//!
//! Un test en jeu fait sur ce PC dans les mêmes conditions (taille de l'écran) remplace la
//! prévision ; un plantage ou un manque de mémoire interdit ce modèle et les plus lourds à cet
//! endroit, quelles que soient les conditions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::catalog::{self, Card, Model, Place, Trial, Trials, Verdict, LOW_PACK};
use crate::hw::{gib, Hw};

// --- Référence ---------------------------------------------------------------------------------

/// Bande passante mémoire de la carte de référence (Radeon RX 6950 XT), en Go/s.
pub const REF_BANDWIDTH: f64 = 576.0;
/// Écran et images par seconde de référence : 3840x1600, qualité maximale, sans traduction.
pub const REF_PIXELS: f64 = 3840.0 * 1600.0;
pub const REF_FPS: f64 = 41.5;
/// Rythme des tests en jeu : une traduction toutes les 3 s.
pub const TEST_RATE: f64 = 20.0;
/// Besoin retenu : un message à traduire toutes les 5 s (chat très animé).
pub const DESIGN_RATE: f64 = 12.0;
/// Fils de calcul des tests sur le processeur de référence (Ryzen 7 5800X3D, 8 cœurs).
pub const REF_CPU_THREADS: f64 = 4.0;

/// Mesures du PC de référence pour un modèle (3840x1600 non plafonné, une traduction toutes
/// les 3 s, mode doux et rafales sur la carte).
#[derive(Debug)]
pub struct Reference {
    pub id: &'static str,
    /// Latence médiane sur la carte, en secondes.
    pub gpu_latency: f64,
    /// Baisse moyenne d'images par seconde au rythme du test.
    pub gpu_drop: f64,
    /// D'où viennent ces valeurs.
    pub gpu_note: &'static str,
    /// Latence et baisse sur le processeur (4 fils) ; `None` : trop lourd pour le processeur.
    pub cpu: Option<(f64, f64)>,
    pub cpu_note: &'static str,
}

pub static REFERENCES: [Reference; 3] = [
    // Série du 09/11/2026 en 0.8.8 (3840x1600, qualité maximale, 40 à 42 images/s sans
    // traduction). Les prévisions de la 0.8.8 s'en écartaient de 12 % au plus.
    Reference {
        id: "eurollm-1.7b",
        gpu_latency: 0.69,
        gpu_drop: 0.016,
        gpu_note: "mesuré le 09/11/2026",
        // 0,83 s au test précédent : 0,95 s retenu (le plus lent).
        cpu: Some((0.95, 0.006)),
        cpu_note: "mesuré le 09/11/2026",
    },
    Reference {
        id: "translategemma-4b",
        gpu_latency: 1.42,
        gpu_drop: 0.041,
        gpu_note: "mesuré le 09/11/2026",
        // Aucune baisse mesurable sur le processeur (−1,2 %, dans le bruit) : 0,7 % retenu,
        // comme le pack low.
        cpu: Some((2.88, 0.007)),
        cpu_note: "mesuré le 09/11/2026",
    },
    Reference {
        id: "salamandrata-7b",
        gpu_latency: 1.27,
        gpu_drop: 0.061,
        gpu_note: "mesuré le 09/11/2026",
        cpu: None,
        cpu_note: "trop lent sur le processeur",
    },
];

pub fn reference(id: &str) -> Option<&'static Reference> {
    REFERENCES.iter().find(|r| r.id == id)
}

// --- Puissance de la carte ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct GpuPower {
    pub name: String,
    /// Bande passante mémoire, en Go/s.
    pub bandwidth: f64,
    /// Valeur de la table (sinon estimée d'après la mémoire vidéo).
    pub known: bool,
    pub laptop: bool,
    /// Rendement de llama.cpp (Vulkan) à côté du jeu, par rapport à la référence (AMD).
    pub efficiency: f64,
}

impl GpuPower {
    /// Bande passante utile, comparable à celle de la référence.
    pub fn effective(&self) -> f64 {
        self.bandwidth * self.efficiency * if self.laptop { 0.75 } else { 1.0 }
    }
}

/// Classe de carte : cartes de performances théoriques équivalentes, d'après leur bande passante
/// mémoire utile (rendement de llama.cpp et carte portable compris). Toutes les cartes d'une
/// classe reçoivent la même prévision, calculée avec le bas de la classe (prudent pour celles du
/// haut) ; seule la mémoire vidéo, limite dure, est comptée carte par carte.
#[derive(Debug, PartialEq)]
pub struct GpuClass {
    pub name: &'static str,
    /// Bas de la classe, en Go/s.
    pub min: f64,
    /// Bande passante retenue pour les calculs.
    pub rep: f64,
    pub examples: &'static str,
}

pub static GPU_CLASSES: [GpuClass; 7] = [
    GpuClass { name: "S", min: 1000.0, rep: 1000.0, examples: "RTX 4090, RTX 5090, RTX 3090 Ti" },
    GpuClass {
        name: "A",
        min: 750.0,
        rep: 750.0,
        examples: "RTX 5080, RX 7900 XTX, RTX 3090, RTX 3080 Ti, RTX 5070 Ti, RX 7900 XT, RTX 3080",
    },
    GpuClass {
        name: "B",
        min: 560.0,
        rep: 560.0,
        examples: "RTX 4080, RTX 4070 Ti Super, RTX 5070, RX 9070 XT, RX 7800 XT, RX 6950 XT, RX 7900 GRE, RTX 3070 Ti, RTX 2080 Ti",
    },
    GpuClass {
        name: "C",
        min: 420.0,
        rep: 420.0,
        examples: "RX 6800 XT, RX 6900 XT, RTX 4070 Super, RTX 4070, RTX 5060 Ti, RTX 3070, RTX 3060 Ti, RX 7700 XT, RX 6750 XT",
    },
    GpuClass {
        name: "D",
        min: 300.0,
        rep: 300.0,
        examples: "RX 6700 XT, RTX 3060 12 Go, Arc A770, Arc B580, RX 9060 XT, RTX 2060, GTX 1660 Super",
    },
    GpuClass {
        name: "E",
        min: 220.0,
        rep: 220.0,
        examples: "RTX 4060 Ti, RTX 4060, RX 7600, RX 6650 XT, RX 6600, RTX 3060 8 Go, RX 580",
    },
    GpuClass { name: "F", min: 0.0, rep: 150.0, examples: "GTX 1650, RX 6500 XT, Arc A380, cartes portables d'entrée de gamme" },
];

pub fn gpu_class(effective: f64) -> &'static GpuClass {
    GPU_CLASSES.iter().find(|c| effective >= c.min).unwrap_or(&GPU_CLASSES[GPU_CLASSES.len() - 1])
}

/// Classe de processeur : cœurs physiques, bas de la classe retenu (4, 6, 8, 12, 16).
pub fn cpu_class(cores: usize) -> usize {
    [16, 12, 8, 6, 4].into_iter().find(|&c| cores >= c).unwrap_or(cores.max(1))
}

/// Bande passante mémoire des cartes courantes (Go/s, modèles de bureau). Les variantes de
/// mémoire différente se distinguent par la taille de la mémoire vidéo.
const TABLE: &[(&str, f64)] = &[
    // NVIDIA
    ("gtx 1050", 112.0), ("gtx 1050 ti", 112.0), ("gtx 1060", 192.0), ("gtx 1070", 256.0),
    ("gtx 1070 ti", 256.0), ("gtx 1080", 320.0), ("gtx 1080 ti", 484.0), ("gtx 1650", 128.0),
    ("gtx 1650 super", 192.0), ("gtx 1660", 192.0), ("gtx 1660 super", 336.0), ("gtx 1660 ti", 288.0),
    ("rtx 2060", 336.0), ("rtx 2060 super", 448.0), ("rtx 2070", 448.0), ("rtx 2070 super", 448.0),
    ("rtx 2080", 448.0), ("rtx 2080 super", 496.0), ("rtx 2080 ti", 616.0),
    ("rtx 3050", 224.0), ("rtx 3060", 360.0), ("rtx 3060 ti", 448.0), ("rtx 3070", 448.0),
    ("rtx 3070 ti", 608.0), ("rtx 3080", 760.0), ("rtx 3080 ti", 912.0), ("rtx 3090", 936.0),
    ("rtx 3090 ti", 1008.0), ("rtx 4060", 272.0), ("rtx 4060 ti", 288.0), ("rtx 4070", 504.0),
    ("rtx 4070 super", 504.0), ("rtx 4070 ti", 504.0), ("rtx 4070 ti super", 672.0),
    ("rtx 4080", 717.0), ("rtx 4080 super", 736.0), ("rtx 4090", 1008.0), ("rtx 5060", 448.0),
    ("rtx 5060 ti", 448.0), ("rtx 5070", 672.0), ("rtx 5070 ti", 896.0), ("rtx 5080", 960.0),
    ("rtx 5090", 1792.0),
    // AMD
    ("rx 470", 211.0), ("rx 480", 256.0), ("rx 570", 224.0), ("rx 580", 256.0), ("rx 590", 256.0),
    ("rx 5500 xt", 224.0), ("rx 5600 xt", 288.0), ("rx 5700", 448.0), ("rx 5700 xt", 448.0),
    ("rx 6400", 128.0), ("rx 6500 xt", 144.0), ("rx 6600", 224.0), ("rx 6600 xt", 256.0),
    ("rx 6650 xt", 280.0), ("rx 6700", 320.0), ("rx 6700 xt", 384.0), ("rx 6750 xt", 432.0),
    ("rx 6800", 512.0), ("rx 6800 xt", 512.0), ("rx 6900 xt", 512.0), ("rx 6950 xt", 576.0),
    ("rx 7600", 288.0), ("rx 7600 xt", 288.0), ("rx 7700 xt", 432.0), ("rx 7800 xt", 624.0),
    ("rx 7900 gre", 576.0), ("rx 7900 xt", 800.0), ("rx 7900 xtx", 960.0), ("rx 9060 xt", 320.0),
    ("rx 9070", 640.0), ("rx 9070 xt", 640.0),
    // Intel
    ("arc a380", 186.0), ("arc a580", 512.0), ("arc a750", 512.0), ("arc a770", 560.0),
    ("arc b570", 380.0), ("arc b580", 456.0),
];

/// Clé de la table à partir du nom donné par Windows (« NVIDIA GeForce RTX 4070 SUPER »,
/// « AMD Radeon RX 6950 XT », « Intel(R) Arc(TM) A770 Graphics »), et carte portable.
pub fn gpu_key(name: &str) -> Option<(String, bool)> {
    let n = name.to_lowercase().replace("(tm)", " ").replace("(r)", " ").replace(['(', ')', '®', '™', ','], " ");
    let laptop = ["laptop", "mobile", "max-q", "notebook"].iter().any(|k| n.contains(k));
    let words: Vec<&str> = n.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if !matches!(*w, "rtx" | "gtx" | "rx" | "arc") {
            continue;
        }
        let Some(num) = words.get(i + 1) else { continue };
        // Cartes portables AMD : « RX 6800M ».
        let (num, m) = match num.strip_suffix('m') {
            Some(n) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => (n, true),
            _ => (*num, false),
        };
        if !num.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let mut key = vec![w.to_string(), num.to_string()];
        for s in words.iter().skip(i + 2).take(2) {
            if matches!(*s, "ti" | "super" | "xt" | "xtx" | "gre") {
                key.push(s.to_string());
            } else {
                break;
            }
        }
        return Some((key.join(" "), laptop || m));
    }
    None
}

pub fn gpu_power(name: &str, vendor_id: u32, vram: u64) -> GpuPower {
    let vram_gb = vram as f64 / (1u64 << 30) as f64;
    let key = gpu_key(name);
    let table = key.as_ref().and_then(|(k, _)| TABLE.iter().find(|(t, _)| *t == k.as_str())).map(|(k, b)| {
        // Variantes à mémoire différente.
        match (*k, vram_gb) {
            ("rtx 3060", v) if v < 10.0 => 240.0,
            ("rtx 3050", v) if v < 7.0 => 168.0,
            ("rtx 3080", v) if v > 11.0 => 912.0,
            ("arc a770", v) if v < 10.0 => 512.0,
            _ => *b,
        }
    });
    let efficiency = match vendor_id {
        0x8086 => 0.7, // Intel Arc : llama.cpp (Vulkan) nettement moins rapide
        _ => 1.0,
    };
    GpuPower {
        name: name.to_string(),
        // Carte inconnue : environ 28 Go/s par Go de mémoire vidéo (8 Go : 224, 16 Go : 448).
        bandwidth: table.unwrap_or_else(|| (28.0 * vram_gb).clamp(112.0, 600.0)),
        known: table.is_some(),
        laptop: key.is_some_and(|(_, l)| l),
        efficiency,
    }
}

// --- Conditions de jeu ---------------------------------------------------------------------------

/// Ce que WoW demande à la carte : taille de l'image, qualité, plafond d'images par seconde.
#[derive(Clone, Debug, PartialEq)]
pub struct Conditions {
    pub size: Option<(u32, u32)>,
    /// Échelle de rendu de WoW (1 : taille de l'écran).
    pub render_scale: f64,
    /// Qualité graphique de WoW, de 1 à 10 ; `None` : inconnue (comptée au maximum).
    pub quality: Option<u8>,
    /// Plafond d'images par seconde ; `None` : non plafonné (la carte est occupée par le jeu).
    pub cap: Option<f64>,
    /// Images par seconde mesurées sans traduction, si un test les a vues.
    pub measured_fps: Option<f64>,
    pub source: String,
    /// Version du jeu (« _retail_ », « _forever_ »...), si connue.
    pub flavor: Option<String>,
}

impl Conditions {
    /// Pire cas : WoW occupe toute la carte (qualité maximale, sans plafond).
    pub fn worst() -> Conditions {
        Conditions {
            size: None,
            render_scale: 1.0,
            quality: None,
            cap: None,
            measured_fps: None,
            source: "pire cas (WoW occupe toute la carte)".into(),
            flavor: None,
        }
    }

    pub fn pixels(&self) -> f64 {
        self.size.map_or(REF_PIXELS, |(w, h)| w as f64 * h as f64) * self.render_scale * self.render_scale
    }

    /// Part du temps de la carte prise par WoW, de 0 à 1.
    pub fn occupancy(&self, bandwidth: f64) -> f64 {
        let Some(cap) = self.cap else { return 1.0 };
        // Images par seconde que la carte ferait sans plafond, d'après la référence.
        let free = REF_FPS * REF_PIXELS / REF_BANDWIDTH * bandwidth
            / (self.pixels() * quality_factor(self.quality) * flavor_load(self.flavor.as_deref()));
        (cap / free).clamp(0.05, 1.0)
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some((w, h)) = self.size {
            parts.push(if (self.render_scale - 1.0).abs() > 0.01 {
                format!("{w}x{h} (échelle de rendu {:.0} %)", self.render_scale * 100.0)
            } else {
                format!("{w}x{h}")
            });
        }
        if let Some(q) = self.quality {
            parts.push(format!("qualité {q}/10"));
        }
        parts.push(match self.cap {
            Some(c) => format!("plafonné à {c:.0} images/s"),
            None => "non plafonné".into(),
        });
        if let Some(f) = self.measured_fps {
            parts.push(format!("{f:.1} images/s sans traduction").replace('.', ","));
        }
        let version = self.flavor.as_deref().map(|f| format!("version {f}, ")).unwrap_or_default();
        format!("{version}{} - {}", parts.join(", "), self.source)
    }
}

/// Coût d'une image de retail par rapport à Forever, à réglages égaux (décors, effets, éclairage) :
/// estimation, remplacée par la mesure dès qu'un test en jeu a été fait sur retail.
pub const RETAIL_LOAD: f64 = 1.5;

fn flavor_load(flavor: Option<&str>) -> f64 {
    match flavor {
        Some(f) if catalog::is_retail(f) => RETAIL_LOAD,
        _ => 1.0,
    }
}

/// Coût d'une image selon la qualité graphique (10 : 1).
fn quality_factor(q: Option<u8>) -> f64 {
    q.map_or(1.0, |q| 0.35 + 0.65 * (q.clamp(1, 10) as f64 - 1.0) / 9.0)
}

/// Part de la charge de la traduction qui se voit sur les images par seconde : toute quand WoW
/// occupe la carte, environ un tiers quand il lui laisse du temps libre (1080p plafonné à 75 :
/// 7B −0,7 à −4,1 % au lieu de −7 %).
pub fn absorbed(u: f64) -> f64 {
    if u >= 0.9 {
        1.0
    } else if u <= 0.6 {
        0.35
    } else {
        0.35 + 0.65 * (u - 0.6) / 0.3
    }
}

/// Allongement du temps de traduction par le partage de la carte (1 quand WoW l'occupe).
pub fn contention(u: f64) -> f64 {
    0.6 + 0.4 * u.clamp(0.0, 1.0)
}

/// Lignes « SET clé "valeur" » de Config.wtf (clés en minuscules).
pub fn parse_config(text: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for l in text.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("SET ").or_else(|| l.strip_prefix("set ")) else { continue };
        let Some((k, v)) = rest.split_once(' ') else { continue };
        m.insert(k.to_lowercase(), v.trim().trim_matches('"').to_string());
    }
    m
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Conditions d'après Config.wtf. `monitor` : taille et fréquence de l'écran principal.
pub fn conditions_from_config(cfg: &HashMap<String, String>, monitor: Option<(u32, u32, u32)>, source: &str) -> Conditions {
    let get = |k: &str| cfg.get(k).map(String::as_str);
    let windowed = get("gxwindow") == Some("1") && get("gxmaximize") != Some("1");
    let key = if windowed { "gxwindowedresolution" } else { "gxfullscreenresolution" };
    let size = get(key).and_then(parse_size).or(monitor.map(|(w, h, _)| (w, h)));
    let render_scale = get("renderscale").and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.1 && *s <= 2.0).unwrap_or(1.0);
    let quality = get("graphicsquality").and_then(|v| v.parse::<u8>().ok()).filter(|q| (1..=10).contains(q));
    let mut cap = get("maxfps")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| *f >= 10.0 && get("usemaxfps") != Some("0"));
    if get("vsync") == Some("1") {
        let hz = monitor.map_or(60.0, |(_, _, hz)| if hz >= 30 { hz as f64 } else { 60.0 });
        cap = Some(cap.map_or(hz, |c| c.min(hz)));
    }
    Conditions { size, render_scale, quality, cap, measured_fps: None, source: source.into(), flavor: None }
}

/// Champ « clé=valeur » du détail d'un essai.
fn field<'a>(detail: &'a str, key: &str) -> Option<&'a str> {
    detail.split([';', ' ']).find_map(|p| p.strip_prefix(key)?.strip_prefix('='))
}

/// Taille d'écran notée par un essai (depuis la 0.8.7).
pub fn trial_size(t: &Trial) -> Option<(u32, u32)> {
    field(&t.detail, "ecran").and_then(parse_size)
}

/// Latence notée par un essai.
pub fn trial_latency(t: &Trial) -> Option<f64> {
    field(&t.detail, "latence").and_then(|v| v.parse().ok()).or_else(|| {
        let i = t.detail.find("latence médiane ")? + "latence médiane ".len();
        t.detail[i..].split_whitespace().next()?.parse().ok()
    })
}

/// Conditions du test en jeu le plus récent qui les a notées.
pub fn conditions_from_trials(trials: &Trials) -> Option<Conditions> {
    let t = trials.values().filter(|t| field(&t.detail, "ref").is_some()).max_by_key(|t| t.when)?;
    let cap = match field(&t.detail, "plafond") {
        None | Some("non") => None,
        Some(c) => c.parse().ok(),
    };
    Some(Conditions {
        size: trial_size(t),
        render_scale: 1.0,
        quality: None,
        cap,
        measured_fps: field(&t.detail, "ref").and_then(|v| v.parse().ok()),
        source: format!("mesurées au test en jeu du {}", date(t.when)),
        flavor: catalog::trial_version(&t.detail).map(String::from),
    })
}

/// Conditions retenues : celles du dernier test si l'écran n'a pas changé depuis, sinon celles
/// de Config.wtf, sinon le pire cas.
pub fn conditions(trials: &Trials, config: Option<Conditions>) -> Conditions {
    let tested = conditions_from_trials(trials);
    match (tested, config) {
        (Some(t), Some(c)) if c.size.is_none() || c.size == t.size => Conditions { quality: c.quality, ..t },
        (Some(t), None) => t,
        (_, Some(c)) => c,
        (None, None) => Conditions::worst(),
    }
}

/// Dossiers de WoW où l'addon est installé (notés par `install`).
fn wow_dirs_file() -> PathBuf {
    crate::fonts::cache_root().join("wow-dossiers.txt")
}

/// Note un dossier de version de WoW (D:\World of Warcraft\_forever_).
pub fn remember_wow(dir: &Path) {
    let file = wow_dirs_file();
    let line = dir.display().to_string();
    let known = std::fs::read_to_string(&file).unwrap_or_default();
    if known.lines().any(|l| l == line) {
        return;
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&file, format!("{known}{line}\n"));
}

/// Config.wtf le plus récent parmi les versions connues (ou celle demandée).
pub fn read_config(only: Option<&Path>) -> Option<Conditions> {
    let dirs: Vec<PathBuf> = match only {
        // Dossier d'une version (…\_forever_), ou dossier de WoW : toutes ses versions.
        Some(d) if d.join("WTF").is_dir() => vec![d.to_path_buf()],
        Some(d) => std::fs::read_dir(d)
            .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("WTF").is_dir()).collect())
            .unwrap_or_default(),
        None => std::fs::read_to_string(wow_dirs_file()).unwrap_or_default().lines().map(PathBuf::from).collect(),
    };
    let (path, _) = dirs
        .iter()
        .map(|d| d.join("WTF").join("Config.wtf"))
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (p, t)))
        .max_by_key(|(_, t)| *t)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let flavor = path.parent()?.parent()?.file_name()?.to_string_lossy().into_owned();
    Some(conditions_from_config(&parse_config(&text), monitor(), &format!("réglages de WoW ({flavor}\\WTF\\Config.wtf)")))
}

/// Taille et fréquence de l'écran principal.
pub fn monitor() -> Option<(u32, u32, u32)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        // Mode actuel de l'écran principal, en pixels réels (indépendant de la mise à l'échelle).
        let ok = unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut dm) } != 0;
        (ok && dm.dmPelsWidth > 0).then_some((dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn date(secs: u64) -> String {
    // Jour/mois/année depuis la date Unix (calendrier civil).
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}/{m:02}/{y}")
}

// --- Prévisions ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    No,
    Borderline,
    Likely,
    Sure,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Sure => "sûr",
            Level::Likely => "probable",
            Level::Borderline => "limite",
            Level::No => "non",
        }
    }
}

/// Classement d'après la latence et la capacité (messages par minute sous 5 % de baisse).
pub fn level(latency: f64, capacity: f64, max_latency: f64) -> Level {
    if latency > max_latency * 1.25 || capacity < DESIGN_RATE * 0.75 {
        Level::No
    } else if latency > max_latency || capacity < DESIGN_RATE {
        Level::Borderline
    } else if latency <= max_latency * 0.8 && capacity >= DESIGN_RATE * 1.5 {
        Level::Sure
    } else {
        Level::Likely
    }
}

/// Capacité : messages par minute traduits en gardant la baisse sous 5 %, d'après la baisse
/// au rythme du test.
pub fn capacity(drop_at_test: f64) -> f64 {
    if drop_at_test <= 0.0005 {
        f64::INFINITY
    } else {
        TEST_RATE * catalog::MAX_FPS_DROP / drop_at_test
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub model: &'static Model,
    pub place: Place,
    /// Mémoire occupée (vidéo sur la carte, vive sur le processeur).
    pub footprint: u64,
    pub latency: f64,
    pub capacity: f64,
    pub level: Level,
    /// Raison du niveau, pour le joueur.
    pub why: String,
    /// « prévision : … » ou « test en jeu du jj/mm/aaaa ».
    pub source: String,
    /// Peut tourner un jour sur ce matériel (carte assez grande à côté d'un WoW léger ;
    /// processeur avec AVX2, assez de cœurs et de mémoire vive), même hors de la chaîne.
    pub possible: bool,
    /// Échec en jeu qui l'interdit ici (plantage ou manque de mémoire, lui ou plus léger), et
    /// son résumé.
    pub banned: Option<(Verdict, String)>,
}

impl Candidate {
    pub fn label(&self) -> String {
        format!("{} {}", self.model.name, self.place.label())
    }

    /// Écart minimal entre deux débuts de traduction pour garder la baisse sous 5 % (0 : aucun).
    pub fn min_gap(&self) -> f64 {
        if self.capacity.is_finite() && self.capacity > 0.0 {
            60.0 / self.capacity
        } else {
            0.0
        }
    }
}

#[derive(Debug)]
pub struct Plan {
    pub gpu: Option<GpuPower>,
    /// Classe de la carte : les calculs prennent sa bande passante retenue.
    pub class: Option<&'static GpuClass>,
    pub card: Option<Card>,
    pub conditions: Conditions,
    /// Part de la carte prise par WoW.
    pub occupancy: f64,
    pub threads: usize,
    pub cores: usize,
    /// Classe du processeur (bas de la classe, en cœurs).
    pub cpu_class: usize,
    pub candidates: Vec<Candidate>,
    /// Chaîne de repli : indices dans `candidates`, du meilleur au plus sûr.
    pub chain: Vec<usize>,
    /// Modèles à télécharger d'office : pack low d'abord, puis tous ceux qui peuvent tourner sur
    /// ce PC et ne sont pas interdits, dans l'ordre de la chaîne puis de la qualité (0.14).
    pub downloads: Vec<&'static Model>,
}

/// Échec constaté en jeu, pendant l'usage normal.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Pilote graphique réinitialisé, ou serveur de traduction arrêté.
    Crash,
    /// Plus assez de mémoire vidéo.
    Memory,
    /// Baisse d'images par seconde ou à-coups au-delà des seuils malgré le régulateur.
    Heavy,
    /// Traductions trop lentes (latence médiane au-delà du seuil).
    Slow,
}

impl Plan {
    pub fn step(&self, pos: usize) -> Option<&Candidate> {
        self.chain.get(pos).map(|&i| &self.candidates[i])
    }

    pub fn best(&self) -> Option<&Candidate> {
        self.step(0)
    }

    /// Meilleur modèle hors pack low (HIGH), s'il est dans la chaîne.
    pub fn high(&self) -> Option<&'static Model> {
        self.best().map(|c| c.model).filter(|m| m.id != LOW_PACK.id)
    }

    #[allow(dead_code)]
    pub fn candidate(&self, id: &str, place: Place) -> Option<&Candidate> {
        self.candidates.iter().find(|c| c.model.id == id && c.place == place)
    }

    /// Meilleure place d'un modèle (celle de la chaîne, sinon la mieux classée).
    pub fn best_for(&self, id: &str) -> Option<&Candidate> {
        self.chain
            .iter()
            .map(|&i| &self.candidates[i])
            .find(|c| c.model.id == id)
            .or_else(|| self.candidates.iter().filter(|c| c.model.id == id).max_by_key(|c| c.level))
    }

    /// Rang d'un modèle dans la chaîne (1 : le meilleur).
    pub fn rank(&self, id: &str) -> Option<usize> {
        self.chain.iter().position(|&i| self.candidates[i].model.id == id).map(|p| p + 1)
    }

    /// Étape suivante de la chaîne après un échec en jeu à l'étape `pos`.
    ///   - plantage ou mémoire sur la carte : la carte est interdite à ce modèle et aux plus
    ///     lourds ; le même modèle sur le processeur reste permis (« plante sur la carte, pas
    ///     sur le processeur ») ;
    ///   - trop lourd pour le jeu : pareil, les plus légers sur la carte restent permis ;
    ///   - trop lent : tout ce qui est prévu aussi lent ou plus lent est sauté.
    #[allow(dead_code)]
    pub fn fallback(&self, pos: usize, f: Failure) -> Option<usize> {
        self.fallback_if(pos, f, &|_| true)
    }

    /// Pareil, parmi les étapes que `keep` accepte (par exemple : modèle installé).
    pub fn fallback_if(&self, pos: usize, f: Failure, keep: &dyn Fn(&Candidate) -> bool) -> Option<usize> {
        let cur = self.step(pos)?;
        (pos + 1..self.chain.len()).find(|&p| {
            let c = &self.candidates[self.chain[p]];
            keep(c)
                && match f {
                    Failure::Crash | Failure::Memory | Failure::Heavy => {
                        !(c.place == cur.place && c.footprint >= cur.footprint)
                    }
                    Failure::Slow => c.latency < cur.latency,
                }
        })
    }
}

/// Tests en jeu qui valent pour les conditions actuelles (même taille d'écran).
fn applies(t: &Trial, cond: &Conditions) -> bool {
    match (trial_size(t), cond.size) {
        (Some(a), Some(b)) => a == b,
        // Essais d'avant la 0.8.7 : conditions inconnues, ils ne valent pas comme mesure.
        _ => false,
    }
}

pub fn estimate(hw: &Hw, measured: &dyn Fn(&str) -> Option<u64>, trials: &Trials, cond: Conditions) -> Plan {
    let card = catalog::card_for(hw, cond.flavor.as_deref());
    let gpu = card.as_ref().and_then(|c| {
        hw.gpus.iter().find(|g| g.name == c.name).map(|g| gpu_power(&g.name, g.vendor_id, g.dedicated))
    });
    let class = gpu.as_ref().map(|g| gpu_class(g.effective()));
    let occupancy = class.map_or(1.0, |c| cond.occupancy(c.rep));
    let cores = hw.cpu.physical.unwrap_or(hw.cpu.logical / 2).max(1);
    let threads = catalog::cpu_threads(hw);
    // Calculs sur le bas de la classe du processeur : 4 fils pour 8 à 11 cœurs, comme la référence.
    let cpu_class = cpu_class(cores);
    let class_threads = (cpu_class / 2).max(2);
    let ram_free = hw.ram_avail.saturating_sub(catalog::RAM_MARGIN);
    let trial = |m: &Model, p: Place| trials.get(&(m.file.to_lowercase(), p));

    // Interdictions : un plantage à un endroit interdit ce modèle et les plus lourds. Un manque de
    // mémoire vidéo n'interdit plus rien (0.14.2) : la place se gère en jeu (module `gouverneur`).
    let mut banned: Vec<(Place, u64, String, Verdict)> = Vec::new();
    for m in catalog::all() {
        for p in [Place::Gpu, Place::Cpu] {
            if let Some(t) = trial(m, p).filter(|t| t.verdict == Verdict::Crash) {
                let (fp, _) = catalog::footprint(m, measured);
                banned.push((p, fp, format!("{} {} : {} le {}", m.name, p.label(), t.summary(), date(t.when)), t.verdict));
            }
        }
    }

    let mut candidates = Vec::new();
    for m in catalog::all() {
        let Some(r) = reference(m.id) else { continue };
        let (fp, _) = catalog::footprint(m, measured);
        // Sur la carte.
        let mut c = Candidate {
            model: m,
            place: Place::Gpu,
            footprint: fp,
            latency: f64::INFINITY,
            capacity: 0.0,
            level: Level::No,
            why: String::new(),
            source: "prévision".into(),
            possible: false,
            banned: None,
        };
        match (&card, class) {
            (Some(cd), Some(cl)) => {
                let s = REF_BANDWIDTH / cl.rep;
                c.latency = r.gpu_latency * s * contention(occupancy);
                c.capacity = capacity(r.gpu_drop * s * absorbed(occupancy));
                c.source = format!("prévision : {}", r.gpu_note);
                // Plus de réserve (0.14.2) : il suffit qu'il tienne à côté d'un WoW réglé au plus
                // bas ; pendant le jeu, il est pris quand la place est là et redescend avant que
                // la mémoire vidéo sature.
                let room = cd.dedicated.saturating_sub(cd.others());
                c.possible = fp + WOW_MIN <= room;
                c.banned = banned.iter().find(|(p, f, _, _)| *p == Place::Gpu && fp >= *f).map(|b| (b.3, b.2.clone()));
                if !c.possible {
                    c.why = format!(
                        "mémoire vidéo : {} pour le modèle et au moins {} pour WoW, la carte en a {}",
                        gib(fp),
                        gib(WOW_MIN),
                        gib(room)
                    );
                } else if let Some((_, _, w, _)) = banned.iter().find(|(p, f, _, _)| *p == Place::Gpu && fp >= *f) {
                    c.why = format!("interdit après un échec : {w}");
                } else {
                    apply_trial(&mut c, trial(m, Place::Gpu), &cond, catalog::MAX_LATENCY);
                    if fp + crate::gouverneur::headroom(cd.dedicated) > cd.free_in_game && c.level >= Level::Likely {
                        c.why += &format!(
                            " ; {} libres estimés pendant le jeu : pris quand la place est là, redescend avant la saturation",
                            gib(cd.free_in_game)
                        );
                    }
                }
            }
            _ => c.why = "pas de carte graphique utilisable".into(),
        }
        candidates.push(c);
        // Sur le processeur.
        let Some((lat, drop)) = r.cpu.filter(|_| m.cpu_ok) else { continue };
        let need = if m.id == LOW_PACK.id { 4 } else { 8 };
        let f = (REF_CPU_THREADS / class_threads as f64).sqrt();
        let ram = catalog::footprint_estimate(m.size);
        let mut c = Candidate {
            model: m,
            place: Place::Cpu,
            footprint: ram,
            latency: lat * f,
            capacity: capacity(drop * (8.0 / cpu_class as f64).max(1.0)),
            level: Level::No,
            why: String::new(),
            source: format!("prévision : {}", r.cpu_note),
            possible: hw.cpu.avx2 && cores >= need && ram <= ram_free,
            banned: banned.iter().find(|(p, f, _, _)| *p == Place::Cpu && ram >= *f).map(|b| (b.3, b.2.clone())),
        };
        if !hw.cpu.avx2 {
            c.why = "processeur sans AVX2".into();
        } else if cores < need {
            c.why = format!("{cores} cœurs, il en faut {need}");
        } else if ram > ram_free {
            c.why = format!("mémoire vive : {} nécessaires, {} libres", gib(ram), gib(ram_free));
        } else if let Some((_, _, w, _)) = banned.iter().find(|(p, f, _, _)| *p == Place::Cpu && ram >= *f) {
            c.why = format!("interdit après un échec : {w}");
        } else {
            apply_trial(&mut c, trial(m, Place::Cpu), &cond, catalog::MAX_LATENCY);
        }
        candidates.push(c);
    }

    // Chaîne : sûrs et probables, meilleure qualité d'abord, puis le plus rapide.
    let mut chain: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].level >= Level::Likely).collect();
    chain.sort_by(|&a, &b| {
        let (a, b) = (&candidates[a], &candidates[b]);
        b.model.quality.cmp(&a.model.quality).then(a.latency.total_cmp(&b.latency))
    });
    // Toujours un repli : le pack low, même limite, s'il n'est pas déjà là.
    if !chain.iter().any(|&i| candidates[i].model.id == LOW_PACK.id) {
        let mut low: Vec<usize> = (0..candidates.len())
            .filter(|&i| candidates[i].model.id == LOW_PACK.id && candidates[i].level == Level::Borderline)
            .collect();
        low.sort_by(|&a, &b| candidates[a].latency.total_cmp(&candidates[b].latency));
        chain.extend(low);
    }
    // Téléchargements (0.14) : tous les modèles qui peuvent tourner sur ce PC et n'y sont pas
    // interdits ; le pack low d'abord (la traduction démarre dès qu'il est là), puis l'ordre de la
    // chaîne (le meilleur ensuite), puis le reste du meilleur au moins bon.
    let runnable = |m: &Model| candidates.iter().any(|c| c.model.id == m.id && c.possible && c.banned.is_none());
    let mut downloads: Vec<&'static Model> = Vec::new();
    let mut add = |m: &'static Model| {
        if runnable(m) && !downloads.iter().any(|d| d.id == m.id) {
            downloads.push(m);
        }
    };
    add(&LOW_PACK);
    for &i in &chain {
        add(candidates[i].model);
    }
    let mut rest: Vec<&'static Model> = catalog::all().collect();
    rest.sort_by_key(|m| std::cmp::Reverse(m.quality));
    for m in rest {
        add(m);
    }
    Plan { gpu, class, card, conditions: cond, occupancy, threads, cores, cpu_class, candidates, chain, downloads }
}

/// Mémoire vidéo d'un WoW le plus léger possible (réglages bas) : un modèle qui ne tient pas même
/// à côté de lui ne tournera jamais sur cette carte et n'est pas téléchargé.
pub const WOW_MIN: u64 = 2 * crate::hw::GIB;

/// Niveau d'après la prévision, corrigé par un test en jeu de ce PC s'il vaut pour ces conditions.
fn apply_trial(c: &mut Candidate, t: Option<&Trial>, cond: &Conditions, max_lat: f64) {
    if let Some(t) = t.filter(|t| applies(t, cond)) {
        if let Some(l) = trial_latency(t) {
            c.latency = l;
        }
        if let Some(d) = t.drop {
            c.capacity = capacity(d);
        }
        c.source = format!("test en jeu du {}", date(t.when));
        // Un « trop lourd » pour à-coups, ou parce que le modèle chargé ralentit déjà le jeu au
        // repos, reste un échec : ni la latence ni la capacité ne le disent. Les autres sont
        // rejugés avec les seuils actuels (latence et baisse sont notées).
        if t.verdict == Verdict::Heavy && ["à-coups", "ralentit"].iter().any(|k| t.detail.contains(k)) {
            c.level = Level::No;
            c.why = format!("test en jeu : {}", t.summary());
            return;
        }
        let lv = level(c.latency, c.capacity, max_lat);
        c.level = if t.verdict == Verdict::Ok { lv.max(Level::Likely) } else { lv };
    } else {
        c.level = level(c.latency, c.capacity, max_lat);
    }
    c.why = match c.level {
        Level::Sure | Level::Likely => format!(
            "{:.2} s par message, jusqu'à {} messages par minute sous 5 % de baisse",
            c.latency,
            cap_label(c.capacity)
        ),
        _ => {
            let mut v = Vec::new();
            if c.latency > max_lat {
                v.push(format!("{:.2} s par message (seuil {max_lat} s)", c.latency));
            }
            if c.capacity < DESIGN_RATE {
                v.push(format!("{} messages par minute sous 5 % de baisse (il en faut {DESIGN_RATE:.0})", cap_label(c.capacity)));
            }
            v.join(", ")
        }
    }
    .replace('.', ",");
}

fn cap_label(c: f64) -> String {
    if c.is_finite() {
        format!("{:.0}", c.min(999.0))
    } else {
        "999".into()
    }
}

/// Plan pour ce PC : mesures d'empreinte et essais notés, conditions du dernier test ou des
/// réglages de WoW.
pub fn for_this_pc(hw: &Hw, wow: Option<&Path>) -> Plan {
    // Dossier d'une version (…\_retail_) : ses essais, ses réglages, sa mémoire vidéo.
    let flavor = wow
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| n.starts_with('_') && n.ends_with('_'));
    let trials = catalog::read_trials_for(flavor.as_deref());
    let mut cond = conditions(&trials, read_config(wow));
    if flavor.is_some() {
        cond.flavor = flavor;
    }
    estimate(hw, &catalog::measured_footprint, &trials, cond)
}

// --- Présentation ----------------------------------------------------------------------------

pub fn report(p: &Plan, hw: &Hw) -> String {
    let mut s = String::from("Estimation sans test - quel modèle pour ce PC\n\n");
    match (&p.gpu, &p.card, p.class) {
        (Some(g), Some(c), Some(cl)) => s.push_str(&format!(
            "Carte : {} - {} ; bande passante {:.0} Go/s ({}{}{}) ; libres pendant le jeu : {}\n\
             \x20     classe {} (à partir de {:.0} Go/s, calculée à {:.0}) : {}\n",
            g.name,
            gib(c.dedicated),
            g.bandwidth,
            if g.known { "table" } else { "estimée d'après la mémoire" },
            if g.laptop { ", carte portable" } else { "" },
            if g.efficiency < 1.0 { format!(", rendement {:.0} %", g.efficiency * 100.0) } else { String::new() },
            gib(c.free_in_game),
            cl.name,
            cl.min,
            cl.rep,
            cl.examples
        )),
        _ => s.push_str("Carte : aucune carte graphique utilisable (traduction sur le processeur)\n"),
    }
    s.push_str(&format!(
        "Processeur : {} - {} cœurs (classe {} cœurs et plus), {} fils pour la traduction, AVX2 {}\n",
        hw.cpu.name,
        p.cores,
        p.cpu_class,
        p.threads,
        if hw.cpu.avx2 { "oui" } else { "non" }
    ));
    s.push_str(&format!("WoW : {}\n", p.conditions.label()));
    s.push_str(&format!(
        "      la carte est occupée par le jeu à {:.0} % ; seuils : {} s par message (carte et processeur), {DESIGN_RATE:.0} messages par minute sous 5 % de baisse\n\n",
        p.occupancy * 100.0,
        catalog::MAX_LATENCY.to_string().replace('.', ",")
    ));
    s.push_str("  Modèle                         Où            Mémoire   Latence   Capacité   Niveau     Source\n");
    for c in &p.candidates {
        s.push_str(&format!(
            "  {:<30} {:<13} {:>8}  {:>7}  {:>9}   {:<9}  {}\n",
            c.model.name,
            if c.place == Place::Gpu { "carte" } else { "processeur" },
            gib(c.footprint),
            if c.latency.is_finite() { format!("{:.2} s", c.latency).replace('.', ",") } else { "-".into() },
            if c.capacity > 0.0 { format!("{}/min", cap_label(c.capacity)) } else { "-".into() },
            c.level.label(),
            c.source
        ));
        if c.level < Level::Likely && !c.why.is_empty() {
            s.push_str(&format!("      {}\n", c.why));
        }
    }
    s.push_str("\nChaîne de repli :\n");
    if p.chain.is_empty() {
        s.push_str("  aucune : ce PC ne peut pas faire tourner la traduction locale\n");
    }
    for (k, &i) in p.chain.iter().enumerate() {
        let c = &p.candidates[i];
        s.push_str(&format!(
            "  {}. {} ({}){}\n",
            k + 1,
            c.label(),
            c.level.label(),
            if c.min_gap() > 3.0 { format!(" - au plus un message toutes les {:.1} s", c.min_gap()).replace('.', ",") } else { String::new() }
        ));
    }
    let total: u64 = p.downloads.iter().map(|m| m.size).sum();
    s.push_str(&format!(
        "\nÀ télécharger d'office : {} ({})\n",
        p.downloads.iter().map(|m| m.name).collect::<Vec<_>>().join(" + "),
        catalog::go(total)
    ));
    s.push_str(
        "\nEn jeu, sans test : le modèle actif est le premier de la chaîne. Plantage ou manque de mémoire\n\
         sur la carte : ce modèle et les plus lourds n'y reviennent plus, le même modèle passe sur le\n\
         processeur s'il est dans la chaîne ; trop lourd : étape suivante ; trop lent : prochain plus rapide.\n",
    );
    s
}

pub fn run(wow: Option<PathBuf>) -> Result<bool, String> {
    let hw = crate::hw::detect();
    let p = for_this_pc(&hw, wow.as_deref());
    print!("{}", report(&p, &hw));
    Ok(true)
}

/// PC fictifs pour les tests (plan, conseil).
#[cfg(test)]
pub mod tests_support {
    use crate::hw::{Cpu, Gpu, Hw, Runtimes, GIB};

    pub fn pc(name: &str, vendor: u32, vram_gb: u64, cores: usize) -> Hw {
        Hw {
            cpu: Cpu { name: "cpu".into(), logical: cores * 2, physical: Some(cores), avx2: true, ..Default::default() },
            ram_total: 32 * GIB,
            ram_avail: 20 * GIB,
            gpus: vec![Gpu { name: name.into(), vendor_id: vendor, dedicated: vram_gb * GIB, used: Some(13 * GIB / 10), ..Default::default() }],
            runtimes: Runtimes { vulkan: true, ..Default::default() },
            wow: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::{Cpu, Gpu, Runtimes, GIB};

    fn pc(name: &str, vendor: u32, vram_gb: u64, cores: usize) -> Hw {
        super::tests_support::pc(name, vendor, vram_gb, cores)
    }

    #[allow(dead_code)]
    fn pc_old(name: &str, vendor: u32, vram_gb: u64, cores: usize) -> Hw {
        Hw {
            cpu: Cpu { name: "cpu".into(), logical: cores * 2, physical: Some(cores), avx2: true, ..Default::default() },
            ram_total: 32 * GIB,
            ram_avail: 20 * GIB,
            gpus: vec![Gpu {
                name: name.into(),
                vendor_id: vendor,
                dedicated: vram_gb * GIB,
                used: Some(13 * GIB / 10),
                ..Default::default()
            }],
            runtimes: Runtimes { vulkan: true, ..Default::default() },
            wow: vec![],
        }
    }

    fn none(_: &str) -> Option<u64> {
        None
    }

    fn chain(p: &Plan) -> Vec<String> {
        p.chain.iter().map(|&i| format!("{}/{}", p.candidates[i].model.id, p.candidates[i].place.code())).collect()
    }

    fn dl(p: &Plan) -> Vec<&str> {
        p.downloads.iter().map(|m| m.id).collect()
    }

    #[test]
    fn card_names_from_windows() {
        assert_eq!(gpu_key("NVIDIA GeForce RTX 4070 SUPER"), Some(("rtx 4070 super".into(), false)));
        assert_eq!(gpu_key("NVIDIA GeForce RTX 4070 Ti SUPER"), Some(("rtx 4070 ti super".into(), false)));
        assert_eq!(gpu_key("AMD Radeon RX 6950 XT"), Some(("rx 6950 xt".into(), false)));
        assert_eq!(gpu_key("AMD Radeon RX 7900 XTX"), Some(("rx 7900 xtx".into(), false)));
        assert_eq!(gpu_key("Radeon RX 580 Series"), Some(("rx 580".into(), false)));
        assert_eq!(gpu_key("Intel(R) Arc(TM) A770 Graphics"), Some(("arc a770".into(), false)));
        assert_eq!(gpu_key("NVIDIA GeForce RTX 4060 Laptop GPU"), Some(("rtx 4060".into(), true)));
        assert_eq!(gpu_key("AMD Radeon RX 6800M"), Some(("rx 6800".into(), true)));
        assert_eq!(gpu_key("AMD Radeon(TM) Graphics"), None);
        let g = gpu_power("NVIDIA GeForce RTX 3060", 0x10DE, 12 * GIB);
        assert_eq!((g.bandwidth, g.known), (360.0, true));
        assert_eq!(gpu_power("NVIDIA GeForce RTX 3060", 0x10DE, 8 * GIB).bandwidth, 240.0);
        let g = gpu_power("Carte inconnue", 0x1002, 16 * GIB);
        assert_eq!((g.bandwidth, g.known), (448.0, false));
        assert!((gpu_power("NVIDIA GeForce RTX 4060 Laptop GPU", 0x10DE, 8 * GIB).effective() - 204.0).abs() < 0.1);
        assert!((gpu_power("Intel(R) Arc(TM) B580 Graphics", 0x8086, 12 * GIB).effective() - 319.2).abs() < 0.1);
    }

    #[test]
    fn wow_settings_from_config_wtf() {
        let cfg = parse_config(
            "SET gxWindow \"1\"\nSET gxMaximize \"1\"\nSET gxFullscreenResolution \"3840x1600\"\n\
             SET graphicsQuality \"10\"\nSET maxFPS \"144\"\nSET useMaxFPS \"0\"\nSET renderScale \"1.000000\"\n",
        );
        let c = conditions_from_config(&cfg, Some((3840, 1600, 144)), "test");
        assert_eq!((c.size, c.quality, c.cap), (Some((3840, 1600)), Some(10), None));
        // Fenêtré en 1080p, plafond 60, synchronisation verticale à 144 Hz : plafond 60.
        let cfg = parse_config("SET gxWindow \"1\"\nSET gxMaximize \"0\"\nSET gxWindowedResolution \"1920x1080\"\nSET maxFPS \"60\"\nSET vsync \"1\"\n");
        let c = conditions_from_config(&cfg, Some((2560, 1440, 144)), "test");
        assert_eq!((c.size, c.cap), (Some((1920, 1080)), Some(60.0)));
        // Rien de noté : taille de l'écran, non plafonné.
        let c = conditions_from_config(&HashMap::new(), Some((2560, 1440, 165)), "test");
        assert_eq!((c.size, c.cap, c.quality), (Some((2560, 1440)), None, None));
        // Carte de référence en 1080p plafonné à 75 : WoW prend environ 60 % de la carte.
        let g = gpu_power("AMD Radeon RX 6950 XT", 0x1002, 16 * GIB);
        let c = Conditions { size: Some((1920, 1080)), cap: Some(75.0), ..Conditions::worst() };
        assert!((c.occupancy(g.effective()) - 0.61).abs() < 0.01);
        assert!((absorbed(0.61) - 0.37).abs() < 0.01);
    }

    #[test]
    fn trial_conditions_and_latency() {
        let t = Trial {
            verdict: Verdict::Heavy,
            when: 1_794_228_000,
            drop: Some(0.052),
            detail: "latence médiane 1.57 s (seuil 1.5 s) ; prio=basse;ecran=3840x1600;ref=41.0;plafond=non".into(),
        };
        assert_eq!(trial_latency(&t), Some(1.57));
        assert_eq!(trial_size(&t), Some((3840, 1600)));
        let mut trials = Trials::new();
        trials.insert(("x".into(), Place::Gpu), t);
        let c = conditions_from_trials(&trials).unwrap();
        assert_eq!((c.size, c.cap, c.measured_fps), (Some((3840, 1600)), None, Some(41.0)));
        assert!(c.source.contains("09/11/2026"));
        // Écran changé depuis le test : les réglages de WoW l'emportent.
        let cfg = Conditions { size: Some((1920, 1080)), cap: Some(60.0), ..Conditions::worst() };
        assert_eq!(conditions(&trials, Some(cfg.clone())).size, Some((1920, 1080)));
        assert_eq!(conditions(&trials, None).size, Some((3840, 1600)));
        assert_eq!(conditions(&Trials::new(), None), Conditions::worst());
    }

    #[test]
    fn reference_pc_at_max_quality_gets_the_7b_with_a_progressive_fallback() {
        let h = pc("AMD Radeon RX 6950 XT", 0x1002, 16, 8);
        let cond = Conditions { size: Some((3840, 1600)), ..Conditions::worst() };
        let p = estimate(&h, &none, &Trials::new(), cond);
        assert_eq!(
            chain(&p),
            ["salamandrata-7b/gpu", "translategemma-4b/gpu", "translategemma-4b/cpu", "eurollm-1.7b/gpu", "eurollm-1.7b/cpu"]
        );
        assert_eq!(p.best().unwrap().level, Level::Likely);
        // 0.14 : tout ce qui peut tourner est téléchargé, pack low d'abord, puis la chaîne.
        assert_eq!(dl(&p), ["eurollm-1.7b", "salamandrata-7b", "translategemma-4b"]);
        assert_eq!(p.high().unwrap().id, "salamandrata-7b");
        // 4B sur le processeur : 2,88 s, probable.
        assert_eq!(p.candidate("translategemma-4b", Place::Cpu).unwrap().level, Level::Likely);
        // Repli en jeu : le 7B plante sur la carte -> 4B, plus léger, toujours sur la carte ;
        // le 4B plante sur la carte -> le 4B sur le processeur (plante sur la carte, pas sur
        // le processeur) ; puis le pack low sur la carte, puis sur le processeur.
        assert_eq!(p.fallback(0, Failure::Crash), Some(1));
        assert_eq!(p.fallback(1, Failure::Crash), Some(2));
        assert_eq!(p.fallback(2, Failure::Crash), Some(3));
        assert_eq!(p.fallback(3, Failure::Crash), Some(4));
        // Le 4B trop lourd pour le jeu sur la carte : le 4B sur le processeur ne charge pas la carte.
        assert_eq!(p.fallback(1, Failure::Heavy), Some(2));
        // Trop lent : le prochain plus rapide (4B, 1,42 s, est plus lent que le 7B : sauté).
        assert_eq!(p.fallback(0, Failure::Slow), Some(3));
        assert!(p.fallback(4, Failure::Heavy).is_none());
        // Classe B, calculée à 560 Go/s : le 4B est sûr sur la carte (1,46 s, 24 par minute).
        assert_eq!(p.class.unwrap().name, "B");
        assert_eq!(p.candidate("translategemma-4b", Place::Gpu).unwrap().level, Level::Sure);
        // Espacement pour le 7B : environ 3,8 s entre deux messages.
        assert!((p.best().unwrap().min_gap() - 3.77).abs() < 0.05);
    }

    #[test]
    fn same_pc_at_1080p_capped_is_comfortable() {
        let h = pc("AMD Radeon RX 6950 XT", 0x1002, 16, 8);
        let cond = Conditions { size: Some((1920, 1080)), cap: Some(75.0), ..Conditions::worst() };
        let p = estimate(&h, &none, &Trials::new(), cond);
        let best = p.best().unwrap();
        assert_eq!((best.model.id, best.level), ("salamandrata-7b", Level::Sure));
        // 7B sûr : le 4B, hors de la chaîne, est téléchargé aussi (il peut tourner ici).
        assert_eq!(dl(&p), ["eurollm-1.7b", "salamandrata-7b", "translategemma-4b"]);
    }

    #[test]
    fn smaller_cards() {
        // RTX 3060 12 Go (classe D, 300 Go/s), sans plafond : le 7B ne tient pas à côté de WoW
        // (7 Go) ; le 4B tient tout juste et passe sous le seuil unique de 3 s (2,73 s,
        // 13 messages par minute) : 23 langues ; puis le pack low, aussi rapide sur le
        // processeur (6 cœurs) que sur cette carte.
        let p = estimate(&pc("NVIDIA GeForce RTX 3060", 0x10DE, 12, 6), &none, &Trials::new(), Conditions::worst());
        assert_eq!(p.class.unwrap().name, "D");
        assert_eq!(chain(&p), ["translategemma-4b/gpu", "eurollm-1.7b/cpu", "eurollm-1.7b/gpu"]);
        let c4 = p.candidate("translategemma-4b", Place::Gpu).unwrap();
        assert_eq!(c4.level, Level::Likely);
        assert!(c4.why.contains("2,73 s par message, jusqu'à 13 messages"), "{}", c4.why);
        // Le 7B tient à côté d'un WoW léger mais suit trop peu de chat sur cette carte (9 messages
        // par minute) : téléchargé, hors de la chaîne.
        assert_eq!(dl(&p), ["eurollm-1.7b", "translategemma-4b", "salamandrata-7b"]);
        let c7 = p.candidate("salamandrata-7b", Place::Gpu).unwrap();
        assert!(c7.why.contains("9 messages par minute") && c7.possible && c7.banned.is_none(), "{}", c7.why);
        // Même classe, même prévision : l'Arc B580 (456 Go/s, rendement 70 %) est calculé comme
        // la RTX 3060.
        let b = estimate(&pc("Intel(R) Arc(TM) B580 Graphics", 0x8086, 12, 6), &none, &Trials::new(), Conditions::worst());
        assert_eq!(b.class.unwrap().name, "D");
        assert_eq!(chain(&b), chain(&p));
        // RTX 4070 Super (classe C, 420 Go/s) : 4B sur la carte (1,95 s) avant le 4B sur le
        // processeur (2,88 s), plus lent ; le 7B ne tient pas dans 12 Go à côté de WoW.
        let p = estimate(&pc("NVIDIA GeForce RTX 4070 SUPER", 0x10DE, 12, 8), &none, &Trials::new(), Conditions::worst());
        assert_eq!(p.class.unwrap().name, "C");
        assert_eq!(chain(&p)[..2], ["translategemma-4b/gpu", "translategemma-4b/cpu"]);
        // RTX 4060 Ti 16 Go (classe E) : le 4B serait trop lent (3,7 s) : pack low.
        let p = estimate(&pc("NVIDIA GeForce RTX 4060 Ti", 0x10DE, 16, 6), &none, &Trials::new(), Conditions::worst());
        assert_eq!(p.class.unwrap().name, "E");
        assert_eq!(p.candidate("translategemma-4b", Place::Gpu).unwrap().level, Level::Borderline);
        assert_eq!(dl(&p), ["eurollm-1.7b", "salamandrata-7b", "translategemma-4b"]);
        // RTX 4090 : le 7B est sûr.
        let p = estimate(&pc("NVIDIA GeForce RTX 4090", 0x10DE, 24, 8), &none, &Trials::new(), Conditions::worst());
        assert_eq!((p.best().unwrap().model.id, p.best().unwrap().level), ("salamandrata-7b", Level::Sure));
        // 8 Go : pack low sur le processeur d'abord (plus rapide que sur cette carte), puis sur
        // la carte quand WoW laisse la place (0.14.2 : plus de réserve, gouverneur en jeu).
        let p = estimate(&pc("AMD Radeon RX 6600", 0x1002, 8, 6), &none, &Trials::new(), Conditions::worst());
        assert_eq!(chain(&p), ["eurollm-1.7b/cpu", "eurollm-1.7b/gpu"]);
        assert!(p.candidate("eurollm-1.7b", Place::Gpu).unwrap().why.contains("pris quand la place est là"));
        // 8 Go, 6 cœurs : 4B et 7B tiennent à côté d'un WoW réglé au plus bas : téléchargés.
        assert_eq!(dl(&p), ["eurollm-1.7b", "salamandrata-7b", "translategemma-4b"]);
        assert!(p.candidate("salamandrata-7b", Place::Gpu).unwrap().possible);
        // 6 Go : le 7B ne tiendra jamais (ni la carte, ni le processeur).
        let p = estimate(&pc("NVIDIA GeForce RTX 2060", 0x10DE, 6, 6), &none, &Trials::new(), Conditions::worst());
        assert!(!p.candidate("salamandrata-7b", Place::Gpu).unwrap().possible);
        assert!(!dl(&p).contains(&"salamandrata-7b"));
        // Processeur à 16 cœurs : le 4B sur le processeur est sûr (8 fils : 2,0 s).
        let mut h = pc("AMD Radeon RX 6600", 0x1002, 8, 16);
        h.ram_avail = 40 * GIB;
        let p = estimate(&h, &none, &Trials::new(), Conditions::worst());
        assert_eq!(chain(&p), ["translategemma-4b/cpu", "eurollm-1.7b/cpu", "eurollm-1.7b/gpu"]);
    }

    #[test]
    fn retail_counts_more_video_memory_and_keeps_its_own_tests() {
        let h = pc("AMD Radeon RX 6950 XT", 0x1002, 16, 8);
        let cond = Conditions { size: Some((3840, 1600)), flavor: Some("_retail_".into()), ..Conditions::worst() };
        assert!(cond.label().starts_with("version _retail_"));
        let p = estimate(&h, &none, &Trials::new(), cond.clone());
        // Retail au maximum : WoW compté 12 Go (11,7 mesurés le 09/11/2026) : 2,7 Go libres estimés.
        // Plus de réserve (0.14.2) : 7B et 4B restent en tête de la chaîne, pris en jeu quand la
        // place est là ; près de la saturation, la traduction redescend d'un cran.
        assert_eq!(chain(&p)[..2], ["salamandrata-7b/gpu", "translategemma-4b/gpu"]);
        assert!(p.candidate("salamandrata-7b", Place::Gpu).unwrap().why.contains("pris quand la place est là"));
        assert!(p.candidate("translategemma-4b", Place::Gpu).unwrap().why.contains("2,7 Go libres estimés"));
        // Série du 09/11/2026 sur retail : 4B sur le processeur trop lourd (baisse de 15 %) ; le
        // pack low sur la carte passe avant le pack low sur le processeur.
        let series = "translategemma-4b-it.Q4_K_M.gguf\tcpu\tlourd\t1794265782\t0.1496\tbaisse moyenne d'images par seconde 15.0 % à 19 messages par minute ; 4 fils;espacement=3;latence=2.98;ecran=3840x1600;version=_retail_;ref=41.9;plafond=non\n\
            EuroLLM-1.7B-Instruct.Q4_K_M.gguf\tgpu\tok\t1794265521\t0.0187\tprio=basse;latence=0.39;ecran=3840x1600;version=_retail_;ref=41.9;plafond=non\n\
            EuroLLM-1.7B-Instruct.Q4_K_M.gguf\tcpu\tok\t1794265282\t0.0807\t4 fils;latence=0.92;ecran=3840x1600;version=_retail_;ref=41.8;plafond=non\n";
        let p = estimate(&h, &none, &catalog::parse_trials_for(series, Some("_retail_")), cond);
        assert_eq!(chain(&p), ["salamandrata-7b/gpu", "translategemma-4b/gpu", "eurollm-1.7b/gpu", "eurollm-1.7b/cpu"]);
        // Manque de mémoire vidéo constaté en jeu : plus une interdiction (0.14.2).
        let memory = format!("{series}salamandraTA_7B_v3_q4_k_m.gguf\tgpu\tmemoire\t1794247908\t\ten jeu : plus que 0,6 Go de mémoire vidéo libre;ecran=3840x1600\n");
        let p = estimate(&h, &none, &catalog::parse_trials_for(&memory, Some("_retail_")), Conditions { size: Some((3840, 1600)), flavor: Some("_retail_".into()), ..Conditions::worst() });
        assert_eq!(chain(&p)[0], "salamandrata-7b/gpu");
        assert!(p.candidate("salamandrata-7b", Place::Gpu).unwrap().banned.is_none());
        // Plafonné : une image de retail coûte plus qu'une image de Forever.
        let capped = |f: Option<&str>| Conditions { size: Some((1920, 1080)), cap: Some(60.0), flavor: f.map(String::from), ..Conditions::worst() };
        assert!(capped(Some("_retail_")).occupancy(560.0) > capped(Some("_forever_")).occupancy(560.0));
        // Essais : chacun sa version ; ceux d'avant la 0.11 sont ceux de Forever.
        let text = "a.gguf\tgpu\tok\t1\t0.01\tlatence=1.2\nb.gguf\tgpu\tlourd\t2\t0.06\tversion=_retail_;latence=2.9\n";
        assert_eq!(catalog::parse_trials_for(text, Some("_forever_")).len(), 1);
        let r = catalog::parse_trials_for(text, Some("_retail_"));
        assert!(r.len() == 1 && r.contains_key(&("b.gguf".to_string(), Place::Gpu)));
        assert_eq!(catalog::parse_trials_for(text, None).len(), 2);
    }

    #[test]
    fn tests_on_this_pc_override_the_estimate() {
        let h = pc("AMD Radeon RX 6950 XT", 0x1002, 16, 8);
        let cond = Conditions { size: Some((3840, 1600)), ..Conditions::worst() };
        let mut trials = Trials::new();
        // Le 7B a fait planter le jeu sur la carte : interdit sur la carte (et rien de plus lourd).
        trials.insert(
            (crate::catalog::ON_DEMAND[1].file.to_lowercase(), Place::Gpu),
            Trial { verdict: Verdict::Crash, when: 1, drop: None, detail: "pilote réinitialisé".into() },
        );
        // Le 4B, mesuré trop lent dans ces conditions.
        trials.insert(
            (crate::catalog::ON_DEMAND[0].file.to_lowercase(), Place::Gpu),
            Trial { verdict: Verdict::Heavy, when: 2, drop: Some(0.05), detail: "latence médiane 3.95 s ; ecran=3840x1600;ref=41;plafond=non".into() },
        );
        let p = estimate(&h, &none, &trials, cond.clone());
        // Le 4B reste possible sur le processeur : la carte ne le concerne pas.
        assert_eq!(chain(&p), ["translategemma-4b/cpu", "eurollm-1.7b/gpu", "eurollm-1.7b/cpu"]);
        assert!(p.candidate("salamandrata-7b", Place::Gpu).unwrap().why.contains("interdit"));
        assert_eq!(p.candidate("translategemma-4b", Place::Gpu).unwrap().level, Level::No);
        // Un « trop lourd » d'avant le seuil unique (1,95 s, seuil de 1,5 s à l'époque) est
        // rejugé : il passe ; un « trop lourd » pour à-coups reste un échec.
        let mut t2 = trials.clone();
        let k4 = (crate::catalog::ON_DEMAND[0].file.to_lowercase(), Place::Gpu);
        t2.insert(k4.clone(), Trial { verdict: Verdict::Heavy, when: 3, drop: Some(0.05), detail: "latence médiane 1.95 s (seuil 1.5 s) ; ecran=3840x1600".into() });
        assert!(estimate(&h, &none, &t2, cond.clone()).candidate("translategemma-4b", Place::Gpu).unwrap().level >= Level::Likely);
        t2.insert(k4, Trial { verdict: Verdict::Heavy, when: 4, drop: Some(0.02), detail: "à-coups de 180 ms pendant les traductions (seuil 100 ms) ; latence=1.30;ecran=3840x1600".into() });
        let c = estimate(&h, &none, &t2, cond.clone());
        let c = c.candidate("translategemma-4b", Place::Gpu).unwrap();
        assert_eq!(c.level, Level::No);
        assert!(c.why.contains("à-coups"), "{}", c.why);
        // Un essai d'un autre écran ne vaut pas mesure : la prévision reste.
        let mut trials = Trials::new();
        trials.insert(
            (crate::catalog::ON_DEMAND[0].file.to_lowercase(), Place::Gpu),
            Trial { verdict: Verdict::Heavy, when: 2, drop: Some(0.05), detail: "latence médiane 1.95 s ; ecran=1920x1080".into() },
        );
        let p = estimate(&h, &none, &trials, cond);
        assert_eq!(p.candidate("translategemma-4b", Place::Gpu).unwrap().level, Level::Sure);
    }

    #[test]
    fn dates() {
        assert_eq!(date(1_794_228_120), "09/11/2026");
        assert_eq!(date(0), "01/01/1970");
    }
}
