//! Catalogue des modèles, choix d'après le matériel, modèle actif et téléchargements.
//!
//! L'addon publié (CurseForge) reste léger : aucun modèle dedans. L'application :
//!   - télécharge toujours le pack low (fr, en, es, it, de, ru) et le moteur llama.cpp :
//!     c'est le modèle par défaut, et le repli ;
//!   - propose les autres modèles sur demande, classés par mémoire vidéo occupée ; le HIGH
//!     d'un joueur est le meilleur modèle que son PC fait tourner pendant le jeu ;
//!   - n'active qu'un modèle à la fois ; si le modèle choisi manque ou a échoué au test en
//!     jeu, le pack low prend le relais.
//!
//! Un modèle est autorisé sur la carte graphique s'il laisse au moins 1 Go de mémoire vidéo
//! libre pendant le jeu, et s'il n'a pas fait réinitialiser le pilote graphique ni trop
//! baissé les images par seconde au test en jeu (plantages du 09/11/2026 : réinitialisations
//! du pilote, pas manque de mémoire). Sinon, il peut passer sur le processeur s'il est assez
//! léger. Le classement de qualité est provisoire : le comparatif S4b le confirmera.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::hw::{gib, Hw, GIB};

/// Taille d'un fichier comme l'affiche Windows (1 Go = 1024³ octets), deux décimales.
pub fn go(b: u64) -> String {
    format!("{:.2} Go", b as f64 / GIB as f64).replace('.', ",")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Langs {
    /// Version low : paires entre fr, en, es, it, de, ru.
    Low,
    /// Les 23 langues.
    All,
}

impl Langs {
    pub fn label(self) -> &'static str {
        match self {
            Langs::Low => "6 langues : français, anglais, espagnol, italien, allemand, russe",
            Langs::All => "les 23 langues",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// Taille exacte du fichier, en octets (vérifiée après téléchargement).
    pub size: u64,
    pub langs: Langs,
    pub license: &'static str,
    /// Qualité de traduction attendue, de 1 à 3 (provisoire jusqu'au comparatif S4b).
    pub quality: u8,
    /// Assez léger pour tourner sur le processeur (8 cœurs) si la carte ne convient pas.
    pub cpu_ok: bool,
}

/// Pack low : téléchargé chez tous les joueurs, modèle par défaut et repli.
pub static LOW_PACK: Model = Model {
    id: "eurollm-1.7b",
    name: "EuroLLM 1.7B",
    file: "EuroLLM-1.7B-Instruct.Q4_K_M.gguf",
    url: "https://huggingface.co/mradermacher/EuroLLM-1.7B-Instruct-GGUF/resolve/main/EuroLLM-1.7B-Instruct.Q4_K_M.gguf",
    size: 1_045_157_088,
    langs: Langs::Low,
    license: "Apache 2.0",
    quality: 1,
    cpu_ok: true,
};

/// Modèles 23 langues, téléchargés sur demande, du plus léger au plus lourd.
/// TranslateGemma 12B retiré le 09/11/2026 : peu de PC le font tourner à côté de WoW.
pub static ON_DEMAND: [Model; 2] = [
    Model {
        id: "translategemma-4b",
        name: "TranslateGemma 4B",
        file: "translategemma-4b-it.Q4_K_M.gguf",
        url: "https://huggingface.co/mradermacher/translategemma-4b-it-GGUF/resolve/main/translategemma-4b-it.Q4_K_M.gguf",
        size: 2_489_909_760,
        langs: Langs::All,
        license: "conditions Gemma",
        quality: 2,
        cpu_ok: true,
    },
    Model {
        id: "salamandrata-7b",
        name: "SalamandraTA 7B v3",
        file: "salamandraTA_7B_v3_q4_k_m.gguf",
        url: "https://huggingface.co/BSC-LT/salamandraTA-7B-instruct-GGUF/resolve/main/salamandraTA_7B_v3_q4_k_m.gguf",
        size: 4_850_334_784,
        langs: Langs::All,
        license: "GPL-3.0",
        quality: 3,
        cpu_ok: false,
    },
];

/// Tous les modèles du catalogue, pack low en tête.
pub fn all() -> impl Iterator<Item = &'static Model> {
    std::iter::once(&LOW_PACK).chain(ON_DEMAND.iter())
}

pub fn by_id(id: &str) -> Option<&'static Model> {
    all().find(|m| m.id == id)
}

/// Modèle du catalogue correspondant à un nom de fichier.
pub fn by_file(name: &str) -> Option<&'static Model> {
    all().find(|m| m.file.eq_ignore_ascii_case(name))
}

/// Mémoire vidéo comptée pour WoW en jeu, d'après le PC de référence (Radeon RX 6950 XT 16 Go,
/// 3840x1600 : WoW mesuré jusqu'à 6,0 Go en jeu, 6,9 à 8,2 Go utilisés en tout bureau compris),
/// avec de la marge pour les raids. Retenue pour tous les joueurs ; si WoW lancé en mesure
/// davantage, c'est la mesure qui compte (WoW réduit ou en zone calme en mesure moins : 3,2 Go
/// le 09/11/2026, ce qui ne dit rien des raids).
pub const WOW_VRAM_REFERENCE: u64 = 7 * GIB;
/// Retail (et ses serveurs de test) : décors plus chargés, textures plus lourdes au maximum des
/// réglages. Mesuré le 09/11/2026 (règle 2.4) : 11,7 Go au maximum, 3840x1600, échelle de rendu
/// 200 % (10 Go estimés avant).
pub const WOW_VRAM_RETAIL: u64 = 12 * GIB;

/// Version du jeu de la famille retail (dossiers _retail_, _ptr_, _xptr_, _beta_).
pub fn is_retail(flavor: &str) -> bool {
    matches!(flavor, "_retail_" | "_ptr_" | "_xptr_" | "_beta_")
}

/// Mémoire vidéo comptée pour WoW selon la version du jeu.
pub fn wow_vram_reference(flavor: Option<&str>) -> u64 {
    match flavor {
        Some(f) if is_retail(f) => WOW_VRAM_RETAIL,
        _ => WOW_VRAM_REFERENCE,
    }
}
// 0.14.2 : plus de mémoire vidéo gardée en réserve (1 Go jusqu'à la 0.14.1) ; près de la
// saturation, le modèle redescend d'une étape et remonte quand la place revient (module
// `gouverneur`).
pub const RAM_MARGIN: u64 = 2 * GIB;
/// Seuils du test en jeu.
pub const MAX_FPS_DROP: f64 = 0.05;
/// Latence médiane maximale d'un message, la même sur la carte et sur le processeur (règle 2.2 du
/// 09/11/2026) : la gêne pour le jeu est comptée à part, en messages par minute sous 5 % de baisse.
pub const MAX_LATENCY: f64 = 3.0;

/// Mémoire occupée par un modèle chargé, estimée : fichier + 15 % + 0,5 Go
/// (4 contextes de 512 jetons, tampons de calcul). Remplacée par la mesure dès qu'il y en a une.
pub fn footprint_estimate(size: u64) -> u64 {
    size + size * 15 / 100 + GIB / 2
}

pub fn footprints_file() -> PathBuf {
    crate::fonts::cache_root().join("empreintes.tsv")
}

/// Empreinte en mémoire vidéo mesurée lors d'un test précédent sur ce PC.
pub fn measured_footprint(file: &str) -> Option<u64> {
    let text = std::fs::read_to_string(footprints_file()).ok()?;
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (name, bytes) = (f.next()?, f.next()?);
            name.eq_ignore_ascii_case(file).then(|| bytes.parse::<u64>().ok()).flatten()
        })
        .last()
}

fn append_line(path: &Path, line: &str) {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    text.push_str(line);
    text.push('\n');
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, text);
}

pub fn record_footprint(file: &str, bytes: u64) {
    append_line(&footprints_file(), &format!("{file}\t{bytes}"));
}

/// Empreinte mesurée sur le PC de référence (Vulkan, Radeon RX 6950 XT, série du 09/11/2026 en
/// 0.8.8 : 4 contextes de 512 jetons). Sert aux joueurs qui n'ont encore rien mesuré :
/// l'estimation à partir de la taille du fichier compte 0,6 à 1,5 Go de trop.
pub fn reference_footprint(file: &str) -> Option<u64> {
    Some(match file {
        "EuroLLM-1.7B-Instruct.Q4_K_M.gguf" => 1_165_373_440,
        "translategemma-4b-it.Q4_K_M.gguf" => 2_857_320_448,
        "salamandraTA_7B_v3_q4_k_m.gguf" => 4_618_977_280,
        _ => return None,
    })
}

/// Empreinte à retenir : (octets, mesurée sur ce PC ?). Sans mesure sur ce PC : celle du PC de
/// référence, sinon une estimation d'après la taille du fichier.
pub fn footprint(m: &Model, measured: &dyn Fn(&str) -> Option<u64>) -> (u64, bool) {
    match measured(m.file) {
        Some(b) => (b, true),
        None => (reference_footprint(m.file).unwrap_or_else(|| footprint_estimate(m.size)), false),
    }
}

// --- Essais en jeu -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Gpu,
    Cpu,
}

impl Place {
    pub fn code(self) -> &'static str {
        match self {
            Place::Gpu => "gpu",
            Place::Cpu => "cpu",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Place::Gpu => "sur la carte graphique",
            Place::Cpu => "sur le processeur",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Compatible : ni réinitialisation du pilote, ni baisse d'images par seconde au-delà du seuil.
    Ok,
    /// Images par seconde ou latence hors des seuils.
    Heavy,
    /// Pilote graphique réinitialisé ou WoW arrêté pendant le test.
    Crash,
    /// Arrêt de sécurité : moins de 1 Go de mémoire vidéo libre.
    Memory,
}

impl Verdict {
    pub fn code(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Heavy => "lourd",
            Verdict::Crash => "plantage",
            Verdict::Memory => "memoire",
        }
    }
    pub fn parse(s: &str) -> Option<Verdict> {
        Some(match s {
            "ok" => Verdict::Ok,
            "lourd" => Verdict::Heavy,
            "plantage" => Verdict::Crash,
            "memoire" => Verdict::Memory,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trial {
    pub verdict: Verdict,
    /// Date du test (secondes depuis 1970).
    pub when: u64,
    /// Baisse moyenne d'images par seconde, si mesurée.
    pub drop: Option<f64>,
    pub detail: String,
}

impl Trial {
    pub fn failed(&self) -> bool {
        self.verdict != Verdict::Ok
    }
    pub fn summary(&self) -> String {
        let what = match self.verdict {
            Verdict::Ok => "réussi",
            Verdict::Heavy => "trop lourd",
            Verdict::Crash => "a fait planter le jeu",
            Verdict::Memory => "mémoire vidéo insuffisante",
        };
        if self.detail.is_empty() {
            what.to_string()
        } else {
            format!("{what} ({})", self.detail)
        }
    }
}

pub type Trials = HashMap<(String, Place), Trial>;

pub fn trials_file() -> PathBuf {
    crate::fonts::cache_root().join("essais.tsv")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Lignes « fichier, carte|processeur, verdict, date, baisse, détail » ; la dernière l'emporte.
pub fn parse_trials(text: &str) -> Trials {
    let mut out = Trials::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        let place = match f[1] {
            "gpu" => Place::Gpu,
            "cpu" => Place::Cpu,
            _ => continue,
        };
        let Some(verdict) = Verdict::parse(f[2]) else { continue };
        out.insert(
            (f[0].to_lowercase(), place),
            Trial {
                verdict,
                when: f[3].parse().unwrap_or(0),
                drop: f.get(4).and_then(|d| d.parse().ok()),
                detail: f.get(5).unwrap_or(&"").to_string(),
            },
        );
    }
    out
}

pub fn read_trials() -> Trials {
    parse_trials(&std::fs::read_to_string(trials_file()).unwrap_or_default())
}

/// Version du jeu notée par un essai (« version=_retail_ », depuis la 0.11).
pub fn trial_version(detail: &str) -> Option<&str> {
    detail.split([';', ' ']).find_map(|p| p.strip_prefix("version="))
}

/// Essais d'une version du jeu : ceux qui la nomment ; pour _forever_, aussi ceux d'avant la 0.11
/// (tous faits sur Forever). Sans version : tous.
pub fn parse_trials_for(text: &str, flavor: Option<&str>) -> Trials {
    let Some(f) = flavor else { return parse_trials(text) };
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| match trial_version(l.split('\t').nth(5).unwrap_or("")) {
            Some(v) => v == f,
            None => f == "_forever_",
        })
        .collect();
    parse_trials(&kept.join("\n"))
}

pub fn read_trials_for(flavor: Option<&str>) -> Trials {
    parse_trials_for(&std::fs::read_to_string(trials_file()).unwrap_or_default(), flavor)
}

pub fn record_trial(file: &str, place: Place, t: &Trial) {
    append_line(
        &trials_file(),
        &format!(
            "{file}\t{}\t{}\t{}\t{}\t{}",
            place.code(),
            t.verdict.code(),
            t.when,
            t.drop.map_or(String::new(), |d| format!("{d:.4}")),
            t.detail.replace(['\t', '\n'], " ")
        ),
    );
}

// --- Carte graphique pendant le jeu ----------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Card {
    pub name: String,
    pub dedicated: u64,
    pub wow_running: bool,
    /// Mémoire vidéo comptée pour WoW : la référence, ou sa mesure si elle est plus haute.
    pub wow: u64,
    pub wow_measured: bool,
    /// Mémoire vidéo libre pendant le jeu, avant chargement d'un modèle.
    pub free_in_game: u64,
}

impl Card {
    /// Windows et les autres programmes (hors WoW).
    pub fn others(&self) -> u64 {
        self.dedicated.saturating_sub(self.free_in_game).saturating_sub(self.wow)
    }
}

/// Carte retenue pour la traduction (la plus grosse carte AMD, NVIDIA ou Intel d'au moins 2 Go).
pub fn card(hw: &Hw) -> Option<Card> {
    card_for(hw, None)
}

/// Même chose, WoW compté selon sa version (retail plus gourmand).
pub fn card_for(hw: &Hw, flavor: Option<&str>) -> Option<Card> {
    let reference = wow_vram_reference(flavor);
    let wow_running = !hw.wow.is_empty();
    hw.gpus
        .iter()
        .filter(|g| !g.software && matches!(g.vendor_id, 0x1002 | 0x10DE | 0x8086) && g.dedicated >= 2 * GIB)
        .filter(|_| hw.runtimes.vulkan || hw.runtimes.cuda)
        .max_by_key(|g| g.dedicated)
        .map(|g| {
            let used_now = g.used.unwrap_or(0);
            // WoW compté au moins pour la référence ; plus s'il en utilise davantage.
            let measured = if wow_running { g.used_by_wow } else { None };
            let (wow, wow_measured) = match measured {
                Some(w) if w >= reference => (w, true),
                _ => (reference, false),
            };
            let others = used_now.saturating_sub(measured.unwrap_or(0));
            let used_in_game = others + wow;
            Card {
                name: g.name.clone(),
                dedicated: g.dedicated,
                wow_running,
                wow,
                wow_measured,
                free_in_game: g.dedicated.saturating_sub(used_in_game),
            }
        })
}

/// Fils de calcul pour la traduction sur le processeur pendant le jeu : la moitié des cœurs,
/// le reste pour WoW.
pub fn cpu_threads(hw: &Hw) -> usize {
    (hw.cpu.physical.unwrap_or(hw.cpu.logical / 2) / 2).max(2)
}

// --- Offre : ce que ce PC peut faire tourner ---------------------------------------------------

#[derive(Clone, Debug)]
pub struct Entry {
    pub model: &'static Model,
    pub footprint: u64,
    pub measured: bool,
    /// Où le modèle tournerait ; `None` : pas sur ce PC.
    pub place: Option<Place>,
    /// Pourquoi (tient, ne tient pas, test échoué).
    pub reason: String,
    /// Dernier test en jeu à cet endroit.
    pub trial: Option<Trial>,
    /// Test en jeu échoué sur la carte graphique (même si le modèle passe sur le processeur).
    pub gpu_failed: Option<Trial>,
    pub installed: bool,
}

impl Entry {
    pub fn eligible(&self) -> bool {
        self.place.is_some()
    }
}

#[derive(Debug)]
pub struct Offer {
    pub card: Option<Card>,
    /// Modèles du plus léger au plus lourd en mémoire vidéo (pack low en tête).
    pub entries: Vec<Entry>,
    /// HIGH : le meilleur modèle autorisé sur ce PC (hors pack low).
    pub high: Option<&'static str>,
    /// Moteur à télécharger : « vulkan » (toutes cartes graphiques) ou « cpu ».
    pub runtime: &'static str,
    pub ram_free: u64,
    pub cpu_threads: usize,
    pub notes: Vec<String>,
}

impl Offer {
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.model.id == id)
    }
}

pub fn offer(hw: &Hw, dir: &Path) -> Offer {
    let trials = read_trials();
    offer_with(hw, &|m| installed_in(dir, m), &measured_footprint, &trials)
}

pub fn installed_in(dir: &Path, m: &Model) -> bool {
    std::fs::metadata(dir.join(m.file)).map(|md| md.len()).ok() == Some(m.size)
}

pub fn offer_with(
    hw: &Hw,
    installed: &dyn Fn(&Model) -> bool,
    measured: &dyn Fn(&str) -> Option<u64>,
    trials: &Trials,
) -> Offer {
    let card = card(hw);
    let ram_free = hw.ram_avail.saturating_sub(RAM_MARGIN);
    let cores = hw.cpu.physical.unwrap_or(hw.cpu.logical / 2).max(1);
    let mut notes = Vec::new();
    if let Some(c) = &card {
        if !c.wow_measured {
            notes.push(format!(
                "{} de mémoire vidéo comptés pour WoW (valeur de référence, raids compris) ; le test en jeu confirmera.",
                gib(WOW_VRAM_REFERENCE)
            ));
        }
    }
    let trial = |m: &Model, p: Place| trials.get(&(m.file.to_lowercase(), p)).cloned();
    let mut entries: Vec<Entry> = all()
        .map(|m| {
            let (fp, mes) = footprint(m, measured);
            let gpu_trial = trial(m, Place::Gpu);
            // Manque de mémoire vidéo constaté : plus un échec (0.14.2), la place se gère en jeu.
            let gpu_failed = gpu_trial.clone().filter(|t| t.failed() && t.verdict != Verdict::Memory);
            // 1. Sur la carte : il doit tenir à côté d'un WoW réglé au plus bas, sans échec au test.
            let mut reason = String::new();
            let mut place = None;
            if let Some(c) = &card {
                let room = c.dedicated.saturating_sub(c.others());
                if fp + crate::plan::WOW_MIN > room {
                    reason = format!(
                        "ne tient pas sur cette carte : modèle {} et au moins {} pour WoW, il y a {}",
                        gib(fp),
                        gib(crate::plan::WOW_MIN),
                        gib(room)
                    );
                } else if let Some(t) = &gpu_failed {
                    reason = format!("test en jeu sur la carte graphique : {}", t.summary());
                } else if fp + crate::gouverneur::headroom(c.dedicated) <= c.free_in_game {
                    place = Some(Place::Gpu);
                    reason = format!(
                        "{} libres pendant le jeu ; le modèle en occupe {} ({}) ; il en restera {}",
                        gib(c.free_in_game),
                        gib(fp),
                        if mes { "mesuré sur ce PC" } else { "estimation" },
                        gib(c.free_in_game - fp)
                    );
                } else {
                    place = Some(Place::Gpu);
                    reason = format!(
                        "{} libres estimés pendant le jeu pour un modèle de {} ({}) : utilisé quand WoW laisse la place ; près de la saturation, la traduction redescend d'un cran",
                        gib(c.free_in_game),
                        gib(fp),
                        if mes { "mesuré sur ce PC" } else { "estimation" }
                    );
                }
            }
            // 2. Sinon, sur le processeur s'il est assez léger.
            if place.is_none() && m.cpu_ok {
                let cpu_trial = trial(m, Place::Cpu);
                let need_cores = if m.langs == Langs::Low { 4 } else { 8 };
                if !hw.cpu.avx2 {
                    if card.is_none() {
                        reason = "processeur sans AVX2 et pas de carte graphique utilisable".into();
                    }
                } else if cores < need_cores || footprint_estimate(m.size) > ram_free {
                    if card.is_none() {
                        reason = format!(
                            "processeur trop juste ({cores} cœurs, {} de mémoire vive libres)",
                            gib(ram_free)
                        );
                    }
                } else if let Some(t) = cpu_trial.clone().filter(|t| t.failed()) {
                    reason = format!("{reason} ; sur le processeur : {}", t.summary())
                        .trim_start_matches(" ; ")
                        .to_string();
                } else {
                    place = Some(Place::Cpu);
                    let why = format!(
                        "sur le processeur ({} fils sur {cores} cœurs, plus lent qu'une carte graphique)",
                        (cores / 2).max(2)
                    );
                    reason = if reason.is_empty() { why } else { format!("{reason} ; {why}") };
                }
            }
            let trial = place.and_then(|p| trial(m, p)).or(gpu_trial.clone());
            Entry {
                model: m,
                footprint: fp,
                measured: mes,
                place,
                reason,
                trial,
                gpu_failed,
                installed: installed(m),
            }
        })
        .collect();
    // Classés par mémoire vidéo occupée ; le pack low reste en tête.
    entries.sort_by_key(|e| (e.model.id != LOW_PACK.id, e.footprint));
    let high = entries
        .iter()
        .filter(|e| e.model.id != LOW_PACK.id && e.eligible())
        .max_by_key(|e| (e.model.quality, e.place == Some(Place::Gpu)))
        .map(|e| e.model.id);
    let low = &entries[0];
    if low.place.is_none() {
        notes.push("Ce PC ne peut pas faire tourner la traduction locale (processeur sans AVX2 ou mémoire insuffisante).".into());
    } else if high.is_none() {
        notes.push("Seul le pack low est possible : les messages dans les autres langues resteront affichés sans traduction.".into());
    }
    Offer {
        card: card.clone(),
        entries,
        high,
        runtime: if card.is_some() { "vulkan" } else { "cpu" },
        ram_free,
        cpu_threads: cpu_threads(hw),
        notes,
    }
}

// --- Modèle actif ----------------------------------------------------------------------------

pub fn active_file() -> PathBuf {
    crate::fonts::cache_root().join("modele-actif.txt")
}

pub fn read_active() -> Option<String> {
    std::fs::read_to_string(active_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn set_active(id: &str) -> Result<(), String> {
    by_id(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
    let p = active_file();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, id).map_err(|e| format!("{} : {e}", p.display()))
}

/// Modèle réellement utilisé : le choix du joueur s'il est installé et autorisé sur ce PC,
/// sinon le pack low (avec la raison du repli).
pub fn effective_active<'a>(chosen: Option<&str>, offer: &'a Offer) -> (&'a Entry, Option<String>) {
    let low = &offer.entries[0];
    let Some(id) = chosen else { return (low, None) };
    match offer.entry(id) {
        None => (low, Some(format!("modèle « {id} » inconnu"))),
        Some(e) if e.model.id == LOW_PACK.id => (e, None),
        Some(e) if !e.installed => (low, Some(format!("{} n'est pas installé", e.model.name))),
        Some(e) if !e.eligible() => (low, Some(format!("{} : {}", e.model.name, e.reason))),
        Some(e) => (e, None),
    }
}

// --- Dossiers et moteur ----------------------------------------------------------------------

pub fn models_dir() -> PathBuf {
    crate::fonts::cache_root().join("modeles")
}

pub fn runtime_dir() -> PathBuf {
    crate::fonts::cache_root().join("llama")
}

/// Taille de l'archive du moteur (version b11785, octobre 2026), pour l'annonce.
pub fn runtime_size(runtime: &str) -> u64 {
    if runtime == "vulkan" {
        36_804_732
    } else {
        24_010_888
    }
}

/// Dossier d'un moteur déjà décompressé (« vulkan », « cpu », « hip-radeon »).
pub fn runtime_build(dir: &Path, runtime: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let suffix = format!("-bin-win-{runtime}-x64");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(exe).is_file())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)))
        .collect();
    found.sort();
    found.pop()
}

pub fn runtime_present(dir: &Path, runtime: &str) -> bool {
    runtime_build(dir, runtime).is_some()
}

/// Espace libre sur le disque d'un dossier (Windows).
pub fn free_disk(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut probe = dir.to_path_buf();
        while !probe.exists() {
            probe = probe.parent()?.to_path_buf();
        }
        let w: Vec<u16> = probe.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        None
    }
}

// --- Téléchargements -------------------------------------------------------------------------

/// Programme de téléchargement : curl.exe (livré avec Windows 10 et 11), remplaçable
/// par la variable CT_CURL (tests).
fn curl() -> String {
    std::env::var("CT_CURL").unwrap_or_else(|_| if cfg!(windows) { "curl.exe" } else { "curl" }.into())
}

/// Débit maximal des téléchargements en octets par seconde (0 : aucun), lu au lancement de
/// chaque appel de curl ; changé pendant un téléchargement, celui-ci est repris à ce débit.
pub static RATE_LIMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Nombre d'essais d'un téléchargement : chaque coupure reprend où elle s'était arrêtée.
const ATTEMPTS: usize = 30;
pub const CANCELLED: &str = "téléchargement annulé (il reprendra là où il s'est arrêté)";

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub got: u64,
    pub total: u64,
    /// Octets par seconde (moyenne glissante).
    pub speed: f64,
    pub attempt: usize,
    pub problem: String,
}

/// Un appel de curl vers `part`, suivi de la taille reçue ; annulable.
fn curl_once(
    url: &str,
    part: &Path,
    resume: bool,
    p: &mut Progress,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&Progress),
) -> Result<(bool, Option<i32>, String), String> {
    let mut cmd = Command::new(curl());
    crate::quiet(&mut cmd).args(["-L", "--fail", "-sS", "--speed-limit", "102400", "--speed-time", "30"]);
    // Débit bridé pendant le jeu (téléchargements de fond, 0.14).
    let limit = RATE_LIMIT.load(Ordering::Relaxed);
    if limit > 0 {
        cmd.args(["--limit-rate", &limit.to_string()]);
    }
    if resume {
        cmd.args(["-C", "-"]);
    }
    let mut child = cmd
        .arg("-o")
        .arg(part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl introuvable : {e}"))?;
    // Arrêté avec l'application (0.14.3) : plus de téléchargement orphelin après Quitter ; il
    // reprend au lancement suivant là où il en était.
    crate::llama::tie_to_app(&child);
    let (mut last_len, mut last_t) = (file_len(part), Instant::now());
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CANCELLED.into());
        }
        std::thread::sleep(Duration::from_millis(200));
        let (len, dt) = (file_len(part), last_t.elapsed().as_secs_f64());
        if dt >= 0.5 {
            let inst = len.saturating_sub(last_len) as f64 / dt;
            p.speed = if p.speed == 0.0 { inst } else { p.speed * 0.7 + inst * 0.3 };
            last_len = len;
            last_t = Instant::now();
        }
        p.got = len;
        report(p);
    };
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    p.got = file_len(part);
    report(p);
    Ok((status.success(), status.code(), err.trim().to_string()))
}

/// Télécharge un modèle et vérifie sa taille exacte. Coupure du serveur ou débit tombé sous
/// 100 Ko/s pendant 30 s : nouvel essai qui reprend la partie déjà reçue (fichier .part).
pub fn download_model_with(
    m: &Model,
    dir: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&Progress),
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", m.file));
    let dest = dir.join(m.file);
    if file_len(&dest) == m.size {
        return Ok(());
    }
    let mut p = Progress { total: m.size, ..Default::default() };
    for attempt in 1..=ATTEMPTS {
        p.attempt = attempt;
        let have = file_len(&part);
        if have == m.size {
            break;
        }
        if have > m.size {
            // Fichier partiel plus grand que prévu : il ne peut pas être bon.
            let _ = std::fs::remove_file(&part);
        }
        if attempt > 1 {
            // Pause avant de reprendre (aucune avec le faux curl des tests).
            let pause = if std::env::var_os("CT_CURL").is_some() { 0 } else { 3 };
            for _ in 0..pause * 5 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(CANCELLED.into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let (ok, code, err) = curl_once(m.url, &part, true, &mut p, cancel, report)?;
        if ok && file_len(&part) == m.size {
            break;
        }
        p.problem = match code {
            Some(56) | Some(18) => "connexion coupée par le serveur".into(),
            Some(28) => "débit trop faible pendant 30 s".into(),
            Some(6) | Some(7) => "serveur injoignable (connexion Internet ?)".into(),
            Some(c) if !err.is_empty() => format!("curl {c} : {err}"),
            Some(c) => format!("curl a renvoyé le code {c}"),
            None => "curl interrompu".into(),
        };
        report(&p);
    }
    let got = file_len(&part);
    if got != m.size {
        return Err(format!(
            "{} incomplet après {ATTEMPTS} essais ({} sur {}, dernier problème : {}) : relancer le téléchargement, il reprendra là",
            m.name,
            go(got),
            go(m.size),
            p.problem
        ));
    }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Affichage de la progression dans la console, au plus une fois par seconde.
pub fn console_progress() -> impl FnMut(&Progress) {
    let mut last = Instant::now() - Duration::from_secs(2);
    let mut problem = String::new();
    move |p: &Progress| {
        if p.problem != problem && !p.problem.is_empty() {
            crate::say!("\n  reprise après : {} (essai {}/{ATTEMPTS})", p.problem, p.attempt + 1);
            problem = p.problem.clone();
        }
        if last.elapsed() >= Duration::from_secs(1) || (p.total > 0 && p.got >= p.total) {
            last = Instant::now();
            let pct = if p.total > 0 { p.got as f64 * 100.0 / p.total as f64 } else { 0.0 };
            print!(
                "\r  {pct:5.1} % - {} sur {} - {:.0} Mo/s   ",
                go(p.got),
                go(p.total),
                p.speed / 1_048_576.0
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }
}

pub fn download_model(m: &Model, dir: &Path) -> Result<(), String> {
    crate::say!("Téléchargement de {} ({}) ...", m.name, go(m.size));
    let cancel = AtomicBool::new(false);
    let r = download_model_with(m, dir, &cancel, &mut console_progress());
    crate::say!("");
    r?;
    crate::say!("  {} vérifié ({} octets) : {}", m.name, m.size, dir.join(m.file).display());
    Ok(())
}

/// Dernière version Windows publiée de llama.cpp pour un moteur : (nom, adresse, taille).
fn runtime_asset(runtime: &str) -> Result<(String, String, u64), String> {
    let out = crate::quiet(&mut Command::new(curl()))
        .args(["-sL", "--retry", "5", "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10"])
        .output()
        .map_err(|e| format!("curl introuvable : {e}"))?;
    let rels: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("liste des versions de llama.cpp illisible : {e}"))?;
    let suffix = format!("-bin-win-{runtime}-x64.zip");
    let asset = rels
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| r["assets"].as_array().cloned().unwrap_or_default())
        .find(|a| a["name"].as_str().is_some_and(|n| n.starts_with("llama-") && n.ends_with(&suffix)))
        .ok_or_else(|| format!("aucune version Windows « {runtime} » de llama.cpp trouvée"))?;
    Ok((
        asset["name"].as_str().unwrap_or("llama.zip").to_string(),
        asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        asset["size"].as_u64().unwrap_or(0),
    ))
}

/// Télécharge et décompresse llama.cpp (dernière version publiée pour Windows).
pub fn download_runtime_with(
    runtime: &str,
    dir: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&Progress),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let (name, url, size) = runtime_asset(runtime)?;
    let zip = dir.join(&name);
    let mut p = Progress { total: size, attempt: 1, ..Default::default() };
    let mut done = false;
    for attempt in 1..=5 {
        p.attempt = attempt;
        let _ = std::fs::remove_file(&zip);
        let (ok, code, err) = curl_once(&url, &zip, false, &mut p, cancel, report)?;
        if ok && (size == 0 || file_len(&zip) == size) {
            done = true;
            break;
        }
        p.problem = format!("curl {} {err}", code.unwrap_or(-1));
        report(&p);
    }
    if !done {
        return Err(format!("téléchargement de {name} impossible ({})", p.problem));
    }
    let target = dir.join(name.trim_end_matches(".zip"));
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    // tar.exe (livré avec Windows 10 et 11) sait décompresser les .zip.
    let st = crate::quiet(&mut Command::new("tar")).arg("-xf").arg(&zip).arg("-C").arg(&target).status().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err(format!("décompression de {name} impossible"));
    }
    let _ = std::fs::remove_file(&zip);
    Ok(target)
}

pub fn download_runtime(runtime: &str, dir: &Path) -> Result<PathBuf, String> {
    crate::say!("Téléchargement du moteur llama.cpp ({runtime}) ...");
    let cancel = AtomicBool::new(false);
    let r = download_runtime_with(runtime, dir, &cancel, &mut console_progress());
    crate::say!("");
    let target = r?;
    crate::say!("  moteur prêt : {}", target.display());
    Ok(target)
}

// --- Annonce dans la console -----------------------------------------------------------------

pub fn stars(q: u8) -> String {
    (1..=3).map(|i| if i <= q { '★' } else { '☆' }).collect()
}

/// Ce que le joueur voit avant tout téléchargement (version console de l'interface).
pub fn notice(o: &Offer, dir: &Path, free_disk: Option<u64>, runtime_present: bool, active: Option<&str>) -> String {
    let mut s = String::new();
    s.push_str("Chevaliers Translators - modèles de traduction\n\n");
    match &o.card {
        Some(c) => s.push_str(&format!(
            "Carte graphique : {} ({}). Pendant le jeu : WoW {} ({}), autres programmes {}, libres {}.\n",
            c.name,
            gib(c.dedicated),
            gib(c.wow),
            if c.wow_measured { "mesuré" } else { "référence" },
            gib(c.others()),
            gib(c.free_in_game)
        )),
        None => s.push_str("Pas de carte graphique utilisable : traduction sur le processeur.\n"),
    }
    s.push_str("Au moins 1 Go de mémoire vidéo reste toujours libre pendant le jeu.\n\n");
    let (eff, why) = effective_active(active, o);
    for (i, e) in o.entries.iter().enumerate() {
        let m = e.model;
        let tag = if m.id == LOW_PACK.id {
            "LOW (par défaut, repli)".to_string()
        } else if o.high == Some(m.id) {
            "HIGH (le meilleur pour ce PC)".to_string()
        } else {
            "sur demande".to_string()
        };
        let state = if e.installed {
            "installé".to_string()
        } else if e.eligible() {
            format!("à télécharger, {}", go(m.size))
        } else {
            "non proposé".to_string()
        };
        s.push_str(&format!(
            "  {}. {} {} - {} - {}{}\n     {} ; licence {} ; mémoire vidéo {} ({}) ; {}\n     {}{}\n",
            i + 1,
            stars(m.quality),
            m.name,
            tag,
            state,
            if eff.model.id == m.id { " - ACTIF" } else { "" },
            m.langs.label(),
            m.license,
            gib(e.footprint),
            if e.measured { "mesurée" } else { "estimée" },
            e.place.map_or("ne tourne pas sur ce PC", |p| p.label()),
            e.reason,
            e.trial.as_ref().filter(|t| !e.reason.contains(&t.summary())).map_or(
                if e.trial.is_some() || e.place.is_none() { String::new() } else { " ; pas encore testé en jeu".to_string() },
                |t| format!(
                " ; test en jeu {} : {}",
                if e.place == Some(Place::Cpu) && e.gpu_failed.is_none() { "processeur" } else if e.place == Some(Place::Cpu) { "carte (échec), à refaire sur le processeur" } else { "carte" },
                t.summary()
            ))
        ));
    }
    if let Some(w) = why {
        s.push_str(&format!("\n  Repli sur le pack low : {w}.\n"));
    }
    let mut auto = 0;
    if !o.entries[0].installed {
        auto += LOW_PACK.size;
    }
    if !runtime_present {
        auto += runtime_size(o.runtime);
    }
    s.push_str(&format!(
        "\n  Moteur llama.cpp ({}) et pack low : {}\n",
        o.runtime,
        if auto == 0 { "déjà présents".to_string() } else { go(auto) }
    ));
    s.push_str("  Les modèles conseillés (estimation ci-dessous) sont téléchargés avec ; les autres sur demande.\n");
    s.push_str(&format!("  Dossier : {}\n", dir.display()));
    if let Some(f) = free_disk {
        s.push_str(&format!("  Espace libre sur ce disque : {}\n", gib(f)));
        if f < auto + GIB {
            s.push_str("  ATTENTION : espace insuffisant, libérez de la place ou choisissez un autre dossier.\n");
        }
    }
    for note in &o.notes {
        s.push_str(&format!("\n  {note}"));
    }
    s.push('\n');
    s
}

/// Version console : annonce et estimation, puis, si `download`, le moteur et les modèles
/// conseillés pour ce PC (chaîne de repli, pack low compris ; `only_low` : le pack low seul),
/// et un modèle sur demande.
pub fn run(download: bool, only_low: bool, dir: Option<PathBuf>, model: Option<&str>, runtime: Option<&str>) -> Result<bool, String> {
    let hw = crate::hw::detect();
    let dir = dir.unwrap_or_else(models_dir);
    let o = offer(&hw, &dir);
    let plan = crate::plan::for_this_pc(&hw, None);
    let rdir = runtime_dir();
    let present = runtime_present(&rdir, o.runtime);
    let extra = runtime.filter(|r| *r != o.runtime && !runtime_present(&rdir, r));
    crate::say!("{}", notice(&o, &dir, free_disk(&dir), present, read_active().as_deref()));
    crate::say!("{}", crate::plan::report(&plan, &hw));
    let wanted = match model {
        Some(id) => {
            let e = o.entry(id).ok_or_else(|| {
                format!("modèle inconnu : {id} (connus : {})", all().map(|m| m.id).collect::<Vec<_>>().join(", "))
            })?;
            if !e.eligible() {
                return Err(format!("{} n'est pas proposé sur ce PC : {}", e.model.name, e.reason));
            }
            Some(e.model)
        }
        None => None,
    };
    if !download {
        crate::say!("(Simulation : ajouter --telecharger pour lancer les téléchargements.)");
        return Ok(true);
    }
    if !present {
        download_runtime(o.runtime, &rdir)?;
    }
    if let Some(r) = extra {
        download_runtime(r, &rdir)?;
    }
    let advised: Vec<&Model> = if only_low { vec![&LOW_PACK] } else { plan.downloads.clone() };
    let mut list: Vec<&Model> = advised;
    if !list.iter().any(|m| m.id == LOW_PACK.id) {
        list.push(&LOW_PACK);
    }
    if let Some(w) = wanted.filter(|w| !list.iter().any(|m| m.id == w.id)) {
        list.push(w);
    }
    for m in list {
        if installed_in(&dir, m) {
            crate::say!("{} : déjà présent et complet.", m.name);
        } else {
            download_model(m, &dir)?;
        }
    }
    crate::say!("\nTout est prêt.");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::{Cpu, Gpu, Runtimes};

    fn pc(vram: u64, used: Option<u64>, wow: Option<u64>, ram_free: u64, avx2: bool) -> Hw {
        Hw {
            cpu: Cpu { name: "t".into(), logical: 16, physical: Some(8), avx2, ..Default::default() },
            ram_total: 64 * GIB,
            ram_avail: ram_free,
            gpus: if vram > 0 {
                vec![Gpu {
                    name: "Radeon".into(),
                    vendor_id: 0x1002,
                    dedicated: vram,
                    used,
                    used_by_wow: wow,
                    ..Default::default()
                }]
            } else {
                vec![]
            },
            runtimes: Runtimes { vulkan: true, ..Default::default() },
            wow: if wow.is_some() { vec!["Wow.exe".into()] } else { vec![] },
        }
    }

    fn none(_: &str) -> Option<u64> {
        None
    }

    fn ids(o: &Offer) -> Vec<(&str, Option<Place>)> {
        o.entries.iter().map(|e| (e.model.id, e.place)).collect()
    }

    #[test]
    fn sorted_by_video_memory_with_relative_high() {
        // PC de référence, WoW en jeu : 16 Go, 6,9 Go utilisés dont 5,6 par WoW, 9,1 Go libres.
        let h = pc(16 * GIB, Some(69 * GIB / 10), Some(56 * GIB / 10), 40 * GIB, true);
        let o = offer_with(&h, &|_| false, &none, &Trials::new());
        assert_eq!(
            ids(&o),
            vec![
                ("eurollm-1.7b", Some(Place::Gpu)),
                ("translategemma-4b", Some(Place::Gpu)),
                ("salamandrata-7b", Some(Place::Gpu))
            ]
        );
        assert_eq!(o.high, Some("salamandrata-7b"));
        assert!(o.entries.windows(2).skip(1).all(|w| w[0].footprint <= w[1].footprint));
        // WoW mesuré à 5,6 Go : compté 7 Go (référence, raids compris) ; 16 - 1,3 - 7 = 7,7 Go libres.
        let c = o.card.as_ref().unwrap();
        assert!(!c.wow_measured && (c.others() as f64 / GIB as f64 - 1.3).abs() < 0.01);
        assert!((c.free_in_game as f64 / GIB as f64 - 7.7).abs() < 0.01);
        // WoW réduit (3,2 Go mesurés, 09/11/2026) : toujours compté 7 Go.
        let h2 = pc(16 * GIB, Some(38 * GIB / 10), Some(32 * GIB / 10), 40 * GIB, true);
        let c2 = card(&h2).unwrap();
        assert_eq!((c2.wow, c2.wow_measured), (WOW_VRAM_REFERENCE, false));
        // WoW au-dessus de la référence (raid, 8 Go) : la mesure compte.
        let h3 = pc(16 * GIB, Some(9 * GIB), Some(8 * GIB), 40 * GIB, true);
        let c3 = card(&h3).unwrap();
        assert_eq!((c3.wow, c3.wow_measured, c3.free_in_game), (8 * GIB, true, 7 * GIB));
        // Carte de 8 Go, WoW non lancé : 0,5 Go + 7 Go de référence, 0,5 Go libre. Plus de réserve
        // (0.14.2) : tout tient à côté d'un WoW réglé au plus bas (2 Go), sur la carte quand WoW
        // laisse la place ; la traduction redescend avant la saturation.
        let h = pc(8 * GIB, Some(GIB / 2), None, 16 * GIB, true);
        let o = offer_with(&h, &|_| false, &none, &Trials::new());
        assert!(o.notes[0].contains("référence"));
        assert_eq!(
            ids(&o),
            vec![
                ("eurollm-1.7b", Some(Place::Gpu)),
                ("translategemma-4b", Some(Place::Gpu)),
                ("salamandrata-7b", Some(Place::Gpu))
            ]
        );
        assert_eq!(o.high, Some("salamandrata-7b"));
        assert!(o.entries[2].reason.contains("quand WoW laisse la place"), "{}", o.entries[2].reason);
        // Carte de 6 Go : le 7B (4,3 Go) ne tient pas même à côté d'un WoW au plus bas.
        let h = pc(6 * GIB, Some(GIB / 2), None, 16 * GIB, true);
        let o = offer_with(&h, &|_| false, &none, &Trials::new());
        assert_eq!(o.entry("salamandrata-7b").unwrap().place, None);
        assert!(o.entry("salamandrata-7b").unwrap().reason.contains("ne tient pas sur cette carte"));
        // Carte de 12 Go, WoW non lancé : 12 - 1 - 7 = 4 Go libres : low et 4B tiennent sans gêner
        // WoW ; le 7B, quand WoW laisse la place.
        let h = pc(12 * GIB, Some(GIB), None, 16 * GIB, true);
        let o = offer_with(&h, &|_| false, &none, &Trials::new());
        assert!(o.entries[0].reason.contains("il en restera"), "{}", o.entries[0].reason);
        assert!(o.entries[1].reason.contains("il en restera"), "{}", o.entries[1].reason);
        assert_eq!(o.entries[2].place, Some(Place::Gpu));
        assert!(o.entries[2].reason.contains("quand WoW laisse la place"), "{}", o.entries[2].reason);
    }

    #[test]
    fn failed_trials_exclude_and_fall_back() {
        let h = pc(16 * GIB, Some(69 * GIB / 10), Some(56 * GIB / 10), 40 * GIB, true);
        let mut t = Trials::new();
        let crash = Trial { verdict: Verdict::Crash, when: 1, drop: Some(0.186), detail: "pilote réinitialisé 3 fois".into() };
        t.insert(("salamandrata_7b_v3_q4_k_m.gguf".into(), Place::Gpu), crash.clone());
        let o = offer_with(&h, &|_| false, &none, &t);
        let e = o.entry("salamandrata-7b").unwrap();
        assert!(!e.eligible() && e.reason.contains("a fait planter le jeu (pilote réinitialisé 3 fois)"), "{}", e.reason);
        assert_eq!(o.high, Some("translategemma-4b"));
        // Le pack low plante sur la carte : il passe sur le processeur.
        t.insert(("eurollm-1.7b-instruct.q4_k_m.gguf".into(), Place::Gpu), crash.clone());
        t.insert(("translategemma-4b-it.q4_k_m.gguf".into(), Place::Gpu), crash.clone());
        let o = offer_with(&h, &|_| false, &none, &t);
        assert_eq!(o.entries[0].place, Some(Place::Cpu));
        assert!(o.entries[0].gpu_failed.is_some());
        assert_eq!(o.entry("translategemma-4b").unwrap().place, Some(Place::Cpu));
        // Échec aussi sur le processeur : le 4B n'est plus proposé, seul le low reste.
        t.insert(
            ("translategemma-4b-it.q4_k_m.gguf".into(), Place::Cpu),
            Trial { verdict: Verdict::Heavy, when: 2, drop: Some(0.09), detail: "baisse 9 %".into() },
        );
        let o = offer_with(&h, &|_| false, &none, &t);
        assert!(o.high.is_none() && o.notes.iter().any(|n| n.contains("Seul le pack low")));
        // Lecture du fichier : la dernière ligne l'emporte.
        let text = "x.gguf\tgpu\tplantage\t10\t0.2\tpilote\nx.gguf\tgpu\tok\t20\t0.01\t\nbad\tline\n";
        let t = parse_trials(text);
        assert_eq!(t.len(), 1);
        assert_eq!(t[&("x.gguf".to_string(), Place::Gpu)].verdict, Verdict::Ok);
    }

    #[test]
    fn measured_footprint_replaces_estimate() {
        // 7,7 Go libres (WoW compté 7 Go) ; 7B estimé 5,7 Go, mesuré 4,3 Go : passe dans les deux cas.
        let h = pc(16 * GIB, Some(69 * GIB / 10), Some(56 * GIB / 10), 40 * GIB, true);
        let measured = |f: &str| (f == "salamandraTA_7B_v3_q4_k_m.gguf").then_some(43 * GIB / 10);
        let o = offer_with(&h, &|_| false, &measured, &Trials::new());
        let e = o.entry("salamandrata-7b").unwrap();
        assert!(e.measured && e.reason.contains("mesuré sur ce PC") && e.reason.contains("il en restera 3,4 Go"), "{}", e.reason);
        // Mesuré à 6,5 Go : il ne reste que 1,2 Go ; plus de réserve de 1 Go en plus (0.14.2).
        let measured = |f: &str| (f == "salamandraTA_7B_v3_q4_k_m.gguf").then_some(65 * GIB / 10);
        let o = offer_with(&h, &|_| false, &measured, &Trials::new());
        let e = o.entry("salamandrata-7b").unwrap();
        assert!(e.eligible() && e.reason.contains("il en restera 1,2 Go"), "{}", e.reason);
        // Mesuré à 7,5 Go : au-delà de la place estimée, pris quand WoW laisse la place.
        let measured = |f: &str| (f == "salamandraTA_7B_v3_q4_k_m.gguf").then_some(75 * GIB / 10);
        let o = offer_with(&h, &|_| false, &measured, &Trials::new());
        let e = o.entry("salamandrata-7b").unwrap();
        assert!(e.eligible() && e.reason.contains("quand WoW laisse la place"), "{}", e.reason);
    }

    #[test]
    fn active_model_falls_back_to_low() {
        let h = pc(16 * GIB, Some(69 * GIB / 10), Some(56 * GIB / 10), 40 * GIB, true);
        let inst = |m: &Model| m.id != "salamandrata-7b";
        let mut t = Trials::new();
        let o = offer_with(&h, &inst, &none, &t);
        assert_eq!(effective_active(None, &o).0.model.id, "eurollm-1.7b");
        assert_eq!(effective_active(Some("translategemma-4b"), &o).0.model.id, "translategemma-4b");
        let (e, why) = effective_active(Some("salamandrata-7b"), &o);
        assert_eq!(e.model.id, "eurollm-1.7b");
        assert!(why.unwrap().contains("pas installé"));
        t.insert(
            ("translategemma-4b-it.q4_k_m.gguf".into(), Place::Gpu),
            Trial { verdict: Verdict::Crash, when: 1, drop: None, detail: String::new() },
        );
        t.insert(
            ("translategemma-4b-it.q4_k_m.gguf".into(), Place::Cpu),
            Trial { verdict: Verdict::Heavy, when: 1, drop: None, detail: String::new() },
        );
        let o = offer_with(&h, &inst, &none, &t);
        let (e, why) = effective_active(Some("translategemma-4b"), &o);
        assert_eq!(e.model.id, "eurollm-1.7b");
        assert!(why.unwrap().contains("planter"));
    }

    /// Faux curl : ajoute un morceau au fichier .part à chaque appel et « coupe » (code 56)
    /// tant que le fichier n'est pas complet ; le téléchargement doit reprendre jusqu'au bout.
    #[cfg(unix)]
    #[test]
    fn download_resumes_after_cuts_and_can_be_cancelled() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ct-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("curl");
        std::fs::write(
            &fake,
            "#!/bin/sh\nout=\"\"\nurl=\"\"\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then out=\"$2\"; fi; url=\"$1\"; shift; done\n\
             case \"$url\" in *lent*) sleep 5;; esac\n\
             head -c 400 /dev/zero >> \"$out\"\nsize=$(wc -c < \"$out\")\nif [ \"$size\" -lt 1000 ]; then exit 56; fi\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("CT_CURL", &fake);
        let m = Model {
            id: "essai",
            name: "Modèle d'essai",
            file: "essai.gguf",
            url: "http://exemple.invalid/essai.gguf",
            size: 1200,
            langs: Langs::Low,
            license: "test",
            quality: 1,
            cpu_ok: true,
        };
        // 400, 800 (coupure), 1200 : complet au 3e essai.
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        download_model_with(&m, &dir, &cancel, &mut |p| seen.push((p.got, p.attempt))).unwrap();
        assert_eq!(file_len(&dir.join("essai.gguf")), 1200);
        assert!(!dir.join("essai.gguf.part").exists());
        assert!(seen.iter().any(|&(g, a)| g == 1200 && a == 3), "{seen:?}");
        // Partiel trop grand : effacé puis retéléchargé.
        std::fs::write(dir.join("autre.gguf.part"), vec![0u8; 5000]).unwrap();
        let m2 = Model { file: "autre.gguf", ..m };
        download_model(&m2, &dir).unwrap();
        assert_eq!(file_len(&dir.join("autre.gguf")), 1200);
        // Annulation pendant un appel lent : arrêt rapide, partie déjà reçue gardée.
        std::fs::write(dir.join("lent.gguf.part"), vec![0u8; 400]).unwrap();
        let m3 = Model { file: "lent.gguf", url: "http://exemple.invalid/lent.gguf", ..m2 };
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.store(true, Ordering::Relaxed);
        });
        let t0 = Instant::now();
        let r = download_model_with(&m3, &dir, &cancel, &mut |_| {});
        t.join().unwrap();
        assert_eq!(r.unwrap_err(), CANCELLED);
        assert!(t0.elapsed() < Duration::from_secs(3));
        assert_eq!(file_len(&dir.join("lent.gguf.part")), 400);
        std::env::remove_var("CT_CURL");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn notice_shows_tiers_and_automatic_downloads() {
        let dir = std::env::temp_dir().join(format!("ct-cat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let h = pc(16 * GIB, Some(69 * GIB / 10), Some(56 * GIB / 10), 40 * GIB, true);
        let o = offer_with(&h, &|m| installed_in(&dir, m), &none, &Trials::new());
        let n = notice(&o, &dir, Some(500 * GIB), false, None);
        assert!(n.contains("★☆☆ EuroLLM 1.7B - LOW (par défaut, repli) - à télécharger, 0,97 Go - ACTIF"), "{n}");
        assert!(n.contains("★★★ SalamandraTA 7B v3 - HIGH (le meilleur pour ce PC)"), "{n}");
        assert!(n.contains("★★☆ TranslateGemma 4B - sur demande"), "{n}");
        // Pack low (0,97 Go) + moteur Vulkan (0,03 Go).
        assert!(n.contains("Moteur llama.cpp (vulkan) et pack low : 1,01 Go"), "{n}");
        // Pack low présent (taille exacte) et moteur présent : plus rien d'office.
        let f = std::fs::File::create(dir.join(LOW_PACK.file)).unwrap();
        f.set_len(LOW_PACK.size).unwrap();
        let o = offer_with(&h, &|m| installed_in(&dir, m), &none, &Trials::new());
        let n = notice(&o, &dir, Some(500 * GIB), true, Some("salamandrata-7b"));
        assert!(n.contains("déjà présents") && n.contains("EuroLLM 1.7B - LOW (par défaut, repli) - installé - ACTIF"), "{n}");
        assert!(n.contains("Repli sur le pack low : SalamandraTA 7B v3 n'est pas installé"), "{n}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runtime_builds_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("ct-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
        for b in ["llama-b12211-bin-win-vulkan-x64", "llama-b12211-bin-win-hip-radeon-x64"] {
            std::fs::create_dir_all(dir.join(b)).unwrap();
            std::fs::write(dir.join(b).join(exe), b"").unwrap();
        }
        assert!(runtime_build(&dir, "vulkan").unwrap().ends_with("llama-b12211-bin-win-vulkan-x64"));
        assert!(runtime_build(&dir, "hip-radeon").unwrap().ends_with("llama-b12211-bin-win-hip-radeon-x64"));
        assert!(runtime_build(&dir, "cpu").is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
