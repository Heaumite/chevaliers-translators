//! Comparatif des modèles de traduction (S4b).
//!
//! Lance `llama-server` (llama.cpp) avec un modèle GGUF, sur la carte graphique ou sur le
//! processeur, lui fait traduire 60 messages de chat de jeu vers 4 langues chacun (240
//! traductions, toutes directes, dont beaucoup sans l'anglais), et mesure : temps de
//! chargement, mémoire vidéo ou vive occupée, latence par message, vitesse de génération.
//! Les traductions sont écrites dans un fichier pour juger la qualité.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::catalog::{Place, Verdict};
use crate::fps::FpsProbe;
use crate::hw::{gib, GIB};
use crate::llama::Tuning;
use crate::burst::Bursts;
use crate::regul::{Action, Regulator};
use std::cell::Cell;

/// (code, nom anglais pour les consignes, nom français pour l'affichage)
pub const LANGS: &[(&str, &str, &str)] = &[
    ("fr", "French", "français"),
    ("en", "English", "anglais"),
    ("de", "German", "allemand"),
    ("es", "Spanish", "espagnol"),
    ("it", "Italian", "italien"),
    ("ru", "Russian", "russe"),
    ("pt", "Portuguese", "portugais"),
    ("nl", "Dutch", "néerlandais"),
    ("pl", "Polish", "polonais"),
    ("cs", "Czech", "tchèque"),
    ("sk", "Slovak", "slovaque"),
    ("hu", "Hungarian", "hongrois"),
    ("ro", "Romanian", "roumain"),
    ("bg", "Bulgarian", "bulgare"),
    ("el", "Greek", "grec"),
    ("hr", "Croatian", "croate"),
    ("sr", "Serbian", "serbe"),
    ("sl", "Slovenian", "slovène"),
    ("uk", "Ukrainian", "ukrainien"),
    ("sv", "Swedish", "suédois"),
    ("no", "Norwegian", "norvégien"),
    ("fi", "Finnish", "finnois"),
    ("da", "Danish", "danois"),
];

pub fn english_name(code: &str) -> &'static str {
    LANGS.iter().find(|(c, _, _)| *c == code).map_or("English", |l| l.1)
}

const CORPUS: &str = include_str!("../bench/corpus.tsv");

#[derive(Clone, Debug)]
pub struct Case {
    pub src: String,
    pub tgt: String,
    pub text: String,
}

/// Langues de la version low : les paires entre elles seulement (30 paires).
pub const LOW: &[&str] = &["fr", "en", "es", "it", "de", "ru"];
/// Messages repris pour la version low (rang dans chaque langue d'origine) : salut et donjon,
/// recherche de groupe, vente d'objet, déconnexion, absence courte en abrégé.
const LOW_PICKS: &[usize] = &[1, 2, 3, 4, 9];

/// Les traductions à faire. `court` : les 2 premiers messages de chaque langue d'origine.
/// `low` : version low, 6 langues, chaque message traduit vers les 5 autres (150 traductions).
pub fn cases(court: bool, low: bool) -> Vec<Case> {
    let mut per_src: Vec<(String, usize)> = Vec::new();
    let mut out = Vec::new();
    for line in CORPUS.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut f = line.splitn(3, '\t');
        let (Some(src), Some(tgts), Some(text)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let low_only = tgts == "low";
        if low_only && !low || low && !LOW.contains(&src) {
            continue;
        }
        let n = match per_src.iter_mut().find(|(s, _)| s == src) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                per_src.push((src.to_string(), 1));
                1
            }
        };
        let targets: Vec<String> = if low {
            if !low_only && !LOW_PICKS.contains(&n) {
                continue;
            }
            LOW.iter().filter(|t| **t != src).map(|t| t.to_string()).collect()
        } else {
            tgts.split(',').map(|t| t.trim().to_string()).collect()
        };
        // Rang parmi les messages retenus de cette langue (pour la version courte).
        let kept = out.iter().filter(|c: &&Case| c.src == src).count() / targets.len().max(1);
        if court && kept >= 2 {
            continue;
        }
        for t in targets {
            out.push(Case {
                src: src.to_string(),
                tgt: t,
                text: text.to_string(),
            });
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Modèle de la famille TranslateGemma : consigne propre au modèle, tours Gemma.
    TranslateGemma,
    /// Modèle SalamandraTA : consigne du modèle, gabarit de discussion du fichier GGUF.
    Salamandra,
    /// Modèle EuroLLM : consigne du modèle, gabarit de discussion du fichier GGUF.
    EuroLlm,
    /// Autre modèle de discussion : consigne générique.
    Chat,
}

impl Format {
    pub fn guess(model: &Path) -> Format {
        let n = model.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        if n.contains("translategemma") {
            Format::TranslateGemma
        } else if n.contains("salamandra") {
            Format::Salamandra
        } else if n.contains("eurollm") {
            Format::EuroLlm
        } else {
            Format::Chat
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::TranslateGemma => "TranslateGemma",
            Format::Salamandra => "SalamandraTA",
            Format::EuroLlm => "EuroLLM",
            Format::Chat => "discussion générique",
        }
    }
}

/// (chemin HTTP, corps JSON) de la requête de traduction.
pub fn request(format: Format, c: &Case) -> (&'static str, Value) {
    let (s, t) = (english_name(&c.src), english_name(&c.tgt));
    match format {
        Format::TranslateGemma => {
            // Consigne du modèle (gabarit de TranslateGemma), en tours Gemma.
            let user = format!(
                "You are a professional {s} ({}) to {t} ({}) translator. Your goal is to accurately convey the meaning and nuances of the original {s} text while adhering to {t} grammar, vocabulary, and cultural sensitivities.\nProduce only the {t} translation, without any additional explanations or commentary. Please translate the following {s} text into {t}:\n\n\n{}",
                c.src, c.tgt, c.text
            );
            let prompt = format!("<start_of_turn>user\n{user}<end_of_turn>\n<start_of_turn>model\n");
            (
                "/completion",
                json!({ "prompt": prompt, "n_predict": 200, "temperature": 0.0, "stop": ["<end_of_turn>"], "cache_prompt": true }),
            )
        }
        Format::Salamandra => {
            let user = format!("Translate the following text from {s} into {t}.\n{s}: {}\n{t}:", c.text);
            (
                "/v1/chat/completions",
                json!({ "messages": [{ "role": "user", "content": user }], "max_tokens": 200, "temperature": 0.0, "cache_prompt": true }),
            )
        }
        Format::EuroLlm => {
            let user = format!("Translate the following {s} source text to {t}:\n{s}: {}\n{t}: ", c.text);
            (
                "/v1/chat/completions",
                json!({ "messages": [{ "role": "user", "content": user }], "max_tokens": 200, "temperature": 0.0, "cache_prompt": true }),
            )
        }
        Format::Chat => {
            let user = format!(
                "Translate this World of Warcraft chat message from {s} into {t}. Keep player names, item names in brackets, numbers and game abbreviations. Reply with the translation only.\n\n{}",
                c.text
            );
            (
                "/v1/chat/completions",
                json!({ "messages": [{ "role": "user", "content": user }], "max_tokens": 200, "temperature": 0.0, "cache_prompt": true }),
            )
        }
    }
}

/// Texte et nombre de jetons générés d'une réponse du serveur.
pub fn parse_reply(v: &Value) -> (String, Option<u64>) {
    let text = v["content"]
        .as_str()
        .or_else(|| v["choices"][0]["message"]["content"].as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let tokens = v["timings"]["predicted_n"]
        .as_u64()
        .or_else(|| v["tokens_predicted"].as_u64())
        .or_else(|| v["usage"]["completion_tokens"].as_u64());
    (text, tokens)
}

/// Retire une étiquette de langue recopiée en tête (« Polish: … ») ; l'application le fera aussi.
/// Débuts de noms de langues (dans plusieurs langues) que les petits modèles mettent en
/// étiquette devant leur réponse : « Francés: », « Русский: », « Italian: »...
const LANGUAGE_STEMS: &[&str] = &[
    "engl", "angl", "ingl", "fran", "deut", "germ", "allem", "alem", "tedes", "niem", "němec", "nemč",
    "span", "espa", "spagn", "ital", "russ", "рус", "polish", "polon", "polsk", "polac", "polni",
    "portug", "dutch", "neder", "néerl", "oland", "holl", "czech", "tchè", "češ", "cesk", "slovak",
    "slovaq", "sloven", "hungar", "hongr", "magyar", "ungar", "roman", "roumain", "român", "rumän",
    "bulgar", "българ", "greek", "grec", "griech", "ελλην", "croat", "hrvat", "kroat", "serb", "srp",
    "српск", "ukrain", "укра", "swed", "suéd", "svensk", "schwed", "norw", "norv", "norsk", "finn",
    "suom", "danish", "danois", "dansk", "dän",
];
/// Mots qui annoncent une traduction : « Translation: », « Traduction du texte russe : »...
const TRANSLATION_WORDS: &[&str] =
    &["transl", "tradu", "tradot", "übersetz", "перев", "tłumacz", "překl", "prevod", "μετάφρασ", "käännös", "oversæt", "översätt"];

/// Retire l'étiquette que certains modèles mettent devant la traduction, et les guillemets
/// ouverts juste après.
pub fn clean(c: &Case, out: &str) -> String {
    let _ = c;
    let mut t = out.trim().to_string();
    for _ in 0..2 {
        let Some(i) = t.find(':') else { break };
        let seg = t[..i].trim();
        let low = seg.to_lowercase();
        let words = seg.split_whitespace().count();
        let is_label = seg.chars().count() <= 70
            && words >= 1
            && ((words <= 3 && LANGUAGE_STEMS.iter().any(|s| low.contains(s)))
                || (words <= 12 && TRANSLATION_WORDS.iter().any(|s| low.contains(s))));
        if !is_label {
            break;
        }
        t = t[i + 1..].trim().to_string();
    }
    // Guillemet ouvrant laissé seul après l'étiquette.
    for (open, close) in [("«", "»"), ("\"", "\""), ("“", "”")] {
        if t.starts_with(open) && !t[open.len()..].contains(close) {
            t = t[open.len()..].trim().to_string();
        }
    }
    t
}

/// Défauts visibles sans lire la langue : vide, recopie, préambule, longueur aberrante.
pub fn flag(c: &Case, out: &str) -> Option<&'static str> {
    let norm = |s: &str| s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    if out.trim().is_empty() {
        return Some("vide");
    }
    if norm(out) == norm(&c.text) && c.src != c.tgt {
        return Some("recopie");
    }
    let low = out.to_lowercase();
    if ["translation:", "here is", "voici la traduction", "übersetzung:", "translate"]
        .iter()
        .any(|p| low.starts_with(p))
    {
        return Some("préambule");
    }
    if out.chars().count() > 3 * c.text.chars().count() + 40 {
        return Some("trop long");
    }
    if out.contains('\n') {
        return Some("plusieurs lignes");
    }
    None
}

// --- HTTP minimal vers le serveur local ------------------------------------------------

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") else { break };
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..eol]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = eol + 2;
        if rest.len() < start + size {
            out.extend_from_slice(&rest[start..]);
            break;
        }
        out.extend_from_slice(&rest[start..start + size]);
        rest = &rest[(start + size + 2).min(rest.len())..];
    }
    out
}

pub fn http(port: u16, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<(u16, String), String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
    s.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| format!("réponse : {e}"))?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("réponse HTTP incomplète")?;
    let headers = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    let body = &raw[split + 4..];
    let body = if headers.contains("transfer-encoding: chunked") {
        dechunk(body)
    } else {
        body.to_vec()
    };
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

// --- Serveur llama.cpp -----------------------------------------------------------------

fn find_server(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        if d.join(name).is_file() {
            return Some(d.join(name));
        }
        if depth < 3 {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.path().is_dir() {
                        stack.push((e.path(), depth + 1));
                    }
                }
            }
        }
    }
    None
}

/// Le serveur est arrêté quoi qu'il arrive (fin, erreur, panique).
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Mémoire vive occupée par le serveur (Windows ; ailleurs : inconnue).
fn ram_of(child: &Child) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id());
            if h.is_null() {
                return None;
            }
            let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ok = GetProcessMemoryInfo(h, &mut c, c.cb) != 0;
            CloseHandle(h);
            ok.then_some(c.PagefileUsage as u64)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        None
    }
}

pub struct Options {
    /// Dossier contenant llama-server ; sinon, le moteur `runtime` installé par l'application.
    pub llama: Option<PathBuf>,
    /// Moteur : « vulkan » (toutes cartes), « hip-radeon » (AMD), « cuda-… » (NVIDIA), « cpu ».
    pub runtime: String,
    pub model: PathBuf,
    pub cpu: bool,
    pub court: bool,
    pub low: bool,
    /// Test en jeu : images par seconde et à-coups mesurés avant et pendant les traductions.
    pub fps: bool,
    /// Ignorer la vérification de la mémoire vidéo (déconseillé : risque de plantage du jeu).
    pub forcer: bool,
    pub port: u16,
    pub out: PathBuf,
    /// Réglages du serveur sur la carte graphique (mode doux par défaut).
    pub tuning: Tuning,
    /// Espacement minimal entre deux traductions, en secondes (3 s par défaut en jeu).
    pub pace: Option<f64>,
    /// Fils de calcul sur le processeur (par défaut : la moitié des cœurs).
    pub threads: Option<usize>,
    /// Régulateur actif pendant le test en jeu sur la carte graphique.
    pub regulate: bool,
    /// Génération par rafales (None : chaque message d'une traite).
    pub bursts: Option<Bursts>,
    /// Avancement, pour l'affichage dans WoW (tests lancés depuis le jeu).
    pub status: Option<std::sync::mpsc::Sender<Status>>,
    /// Arrêt demandé depuis le jeu.
    pub stop: Option<Arc<AtomicBool>>,
    /// Lancé depuis WoW : le jeu est déjà devant, pas de bip ni de retour à la console.
    pub from_game: bool,
}

/// Avancement d'un test, envoyé à l'affichage dans WoW.
#[derive(Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Status {
    Phase(String),
    Translation(String),
    Note(String),
    Done { lines: Vec<String>, verdict: Verdict, recorded: bool, aborted: bool },
}

const ABORTED: &str = "arrêté à la demande";

fn say(o: &Options, s: Status) {
    if let Some(tx) = &o.status {
        let _ = tx.send(s);
    }
}

/// Carte graphique principale (la plus grosse, avec des compteurs de mémoire).
fn main_gpu(hw: &crate::hw::Hw) -> Option<crate::hw::Gpu> {
    hw.gpus.iter().filter(|g| !g.software && g.used.is_some()).max_by_key(|g| g.dedicated).cloned()
}

/// Mémoire vidéo libre en ce moment sur la carte principale (tous programmes compris).
/// Relevé pour les mesures en jeu (0.14) : mémoire vidéo libre, celle de WoW si connue, WoW
/// ouvert ou non.
pub fn vram_sample() -> Option<(u64, Option<u64>, bool)> {
    let hw = crate::hw::detect();
    let running = !hw.wow.is_empty();
    main_gpu(&hw).map(|g| (g.dedicated.saturating_sub(g.used.unwrap_or(0)), g.used_by_wow, running))
}

/// Seuil de saturation de la mémoire vidéo de la carte principale (0.14.2, plus de réserve).
pub fn saturation_now() -> u64 {
    main_gpu(&crate::hw::detect()).map_or(256 << 20, |g| crate::gouverneur::saturation(g.dedicated))
}

pub fn vram_now() -> Option<(String, u64)> {
    main_gpu(&crate::hw::detect()).map(|g| (g.name.clone(), g.dedicated.saturating_sub(g.used.unwrap_or(0))))
}

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn median(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| percentile(v, 0.5))
}

// --- Relevés pendant le test --------------------------------------------------------------

#[derive(Clone, Debug)]
struct Row {
    at: Instant,
    phase: &'static str,
    card_used: Option<u64>,
    server: Option<u64>,
    wow: Option<u64>,
    busy: bool,
}

/// Relevé de la mémoire vidéo chaque seconde, et surveillance de WoW.
struct Monitor {
    stop: Arc<AtomicBool>,
    rows: Arc<Mutex<Vec<Row>>>,
    phase: Arc<Mutex<&'static str>>,
    busy: Arc<AtomicBool>,
    server_pid: Arc<AtomicU32>,
    wow_gone: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Monitor {
    fn start(luid: Option<(i32, u32)>, wow: Vec<u32>, abort: Arc<AtomicBool>) -> Monitor {
        let mut m = Monitor {
            stop: Arc::default(),
            rows: Arc::default(),
            phase: Arc::new(Mutex::new("reference")),
            busy: Arc::default(),
            server_pid: Arc::default(),
            wow_gone: Arc::default(),
            abort,
            handle: None,
        };
        let (stop, rows, phase, busy, pid, gone) =
            (m.stop.clone(), m.rows.clone(), m.phase.clone(), m.busy.clone(), m.server_pid.clone(), m.wow_gone.clone());
        let handle = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let at = Instant::now();
                let p = pid.load(Ordering::Relaxed);
                let s = luid.map(|l| crate::hw::vram_sample(l, (p != 0).then_some(p), &wow)).unwrap_or_default();
                if !wow.is_empty() {
                    let now = crate::hw::wow_pids();
                    if !wow.iter().any(|w| now.contains(w)) {
                        gone.store(true, Ordering::Relaxed);
                    }
                }
                rows.lock().unwrap().push(Row {
                    at,
                    phase: *phase.lock().unwrap(),
                    card_used: s.card_used,
                    server: s.server,
                    wow: s.wow,
                    busy: busy.load(Ordering::Relaxed),
                });
                while at.elapsed() < Duration::from_secs(1) && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        m.handle = Some(handle);
        m
    }
    fn set_phase(&self, p: &'static str) {
        *self.phase.lock().unwrap() = p;
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.lock().unwrap().clone()
    }
    fn wow_gone(&self) -> bool {
        self.wow_gone.load(Ordering::Relaxed)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Mesures de l'addon reçues pendant une phase : (images par seconde, plus longues images).
fn phase_perf(probe: Option<&FpsProbe>, a: Instant, b: Instant) -> (Vec<f64>, Vec<f64>) {
    let s = probe.map(|p| p.timed_between(a, b)).unwrap_or_default();
    (
        s.iter().map(|(_, p)| p.fps as f64).collect(),
        s.iter().filter_map(|(_, p)| p.worst_ms.map(|w| w as f64)).collect(),
    )
}

/// Plafond d'images par seconde (synchronisation verticale, limite de WoW) : référence très
/// stable et collée à une valeur usuelle. Plafonné, WoW laisse du temps libre à la carte et la
/// baisse due à la traduction est sous-estimée par rapport au même PC sans plafond.
pub fn fps_cap(base: &[f64]) -> Option<f64> {
    if base.len() < 6 {
        return None;
    }
    let m = percentile(base, 0.5);
    let spread = percentile(base, 0.95) - percentile(base, 0.05);
    [30.0, 60.0, 72.0, 75.0, 90.0, 100.0, 120.0, 144.0, 165.0, 170.0, 240.0]
        .into_iter()
        .find(|c| (m - c).abs() < 0.6 && spread < 1.5)
}

/// Version du jeu ouverte (« _retail_ », « _forever_ »...), d'après son exécutable (Windows).
pub fn wow_flavor() -> Option<String> {
    #[cfg(windows)]
    {
        crate::capture_win::wow_flavor()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Taille de la zone de jeu de WoW (Windows).
pub fn wow_screen() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        crate::capture_win::wow_client_rect().map(|(l, t, r, b)| (r - l, b - t))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn pct(base: f64, v: f64) -> f64 {
    if base > 0.0 {
        (base - v) / base
    } else {
        0.0
    }
}

/// Attend `d` en surveillant le jeu ; renvoie la raison d'un arrêt.
fn watch_for(d: Duration, mon: &Monitor, srv: &mut Server) -> Option<(Verdict, String)> {
    let t = Instant::now();
    while t.elapsed() < d {
        if let Some(r) = check_alive(mon, srv) {
            return Some(r);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn check_alive(mon: &Monitor, srv: &mut Server) -> Option<(Verdict, String)> {
    if mon.abort.load(Ordering::Relaxed) {
        return Some((Verdict::Ok, ABORTED.into()));
    }
    if mon.wow_gone() {
        return Some((Verdict::Crash, "WoW s'est arrêté pendant le test".into()));
    }
    if let Ok(Some(st)) = srv.0.try_wait() {
        return Some((Verdict::Crash, format!("le serveur de traduction s'est arrêté ({st})")));
    }
    None
}

/// Fait réagir le régulateur aux mesures arrivées depuis `last`.
#[allow(clippy::too_many_arguments)]
fn regulate(
    reg: &mut Regulator,
    probe: &FpsProbe,
    last: &mut Instant,
    windows: &[(Instant, Instant)],
    in_flight: Option<Instant>,
    pid: u32,
    pace: &mut f64,
    bursts: &Cell<Option<Bursts>>,
    notes: &mut Vec<String>,
) -> Option<(Verdict, String)> {
    let now = Instant::now();
    for (ts, p) in probe.timed_between(*last, now) {
        if ts <= *last {
            continue;
        }
        *last = ts;
        let Some(w) = p.worst_ms else { continue };
        // Une mesure couvre la demi-seconde écoulée : traduction en cours pendant cette période ?
        let from = ts.checked_sub(Duration::from_millis(500)).unwrap_or(ts);
        let busy = windows.iter().any(|(a, b)| *a <= ts && *b >= from) || in_flight.is_some_and(|a| a <= ts);
        match reg.observe(w, busy) {
            None => {}
            Some(Action::Adjust { priority, pace: p, bursts: b }) => {
                let r = crate::llama::set_gpu_priority(pid, priority);
                *pace = pace.max(p);
                if bursts.get().is_some() {
                    bursts.set(Some(b));
                }
                let msg = format!(
                    "[régulateur] à-coup de {w:.0} ms (seuil {:.0} ms) : priorité {} sur la carte{}{}, traductions espacées d'au moins {:.0} s",
                    reg.limit_ms,
                    priority.code(),
                    if r.is_ok() { "" } else { " (non appliquée)" },
                    bursts.get().map_or(String::new(), |b| format!(", {}", b.label())),
                    *pace
                );
                crate::say!("{msg}");
                notes.push(msg);
            }
            Some(Action::Restart(t)) => {
                return Some((
                    Verdict::Heavy,
                    format!(
                        "à-coups de {w:.0} ms malgré la priorité minimale (seuil {:.0} ms) ; réglage suivant à essayer : {}",
                        reg.limit_ms,
                        t.label()
                    ),
                ))
            }
            Some(Action::Fallback) => {
                return Some((Verdict::Heavy, format!("à-coups de {w:.0} ms au réglage le plus doux (seuil {:.0} ms)", reg.limit_ms)))
            }
        }
    }
    None
}

/// Traduit un message : découpé en phrases s'il est long, chaque morceau d'une traite ou par
/// rafales. Renvoie (texte, jetons) ; les fenêtres de calcul vont dans `windows`.
#[allow(clippy::too_many_arguments)]
fn translate_message(
    port: u16,
    format: Format,
    c: &Case,
    bursts: &Cell<Option<Bursts>>,
    busy: &AtomicBool,
    windows: &mut Vec<(Instant, Instant)>,
    burst_count: &mut u32,
) -> Result<(String, Option<u64>), String> {
    let mut texts = Vec::new();
    let mut tokens: Option<u64> = None;
    for piece in crate::burst::split_sentences(&c.text, 160) {
        let pc = Case { src: c.src.clone(), tgt: c.tgt.clone(), text: piece };
        let (path, body) = request(format, &pc);
        let raw = match bursts.get() {
            Some(_) => match crate::burst::raw_prompt(port, path, &body) {
                Ok(r) => Some(r),
                Err(e) => {
                    crate::say!("Rafales impossibles avec ce modèle ({e}) : messages traduits d'une traite.");
                    bursts.set(None);
                    None
                }
            },
            None => None,
        };
        let (text, n) = match raw {
            Some((prompt, params)) => {
                let max = body["n_predict"].as_u64().or_else(|| body["max_tokens"].as_u64()).unwrap_or(200) as u32;
                let out = crate::burst::generate(
                    port,
                    &prompt,
                    &params,
                    max,
                    &|| bursts.get().unwrap_or(Bursts { tokens: max, pause_ms: 0 }),
                    Some(busy),
                    &mut |a, b| windows.push((a, b)),
                )?;
                *burst_count += out.bursts;
                (out.text.trim().to_string(), Some(out.tokens))
            }
            None => {
                let t = Instant::now();
                busy.store(true, Ordering::Relaxed);
                let r = http(port, "POST", path, Some(&body), Duration::from_secs(300));
                busy.store(false, Ordering::Relaxed);
                windows.push((t, Instant::now()));
                let (code, b) = r?;
                if code != 200 {
                    return Err(format!("HTTP {code} : {}", b.chars().take(160).collect::<String>()));
                }
                let v: Value = serde_json::from_str(&b).map_err(|e| format!("réponse illisible : {e}"))?;
                parse_reply(&v)
            }
        };
        texts.push(clean(&pc, &text));
        if let Some(n) = n {
            tokens = Some(tokens.unwrap_or(0) + n);
        }
    }
    Ok((texts.join(" "), tokens))
}

pub fn run(o: &Options) -> Result<bool, String> {
    if !o.model.is_file() {
        let name = o.model.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let part = o.model.with_file_name(format!("{name}.part"));
        if part.is_file() {
            let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let of = crate::catalog::by_file(&name).map_or(String::new(), |m| format!(" sur {}", crate::catalog::go(m.size)));
            return Err(format!(
                "modèle pas encore complet ({} reçus{of}) : relancer le téléchargement, il reprendra là",
                crate::catalog::go(have),
            ));
        }
        return Err(format!("modèle introuvable : {}", o.model.display()));
    }
    let server = match &o.llama {
        Some(d) => find_server(d),
        None => {
            let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
            crate::catalog::runtime_build(&crate::catalog::runtime_dir(), &o.runtime).map(|d| d.join(exe))
        }
    }
    .ok_or_else(|| match &o.llama {
        Some(d) => format!("llama-server introuvable dans {}", d.display()),
        None => format!(
            "moteur llama.cpp « {} » absent : « installation --telecharger » (Vulkan) ou « installation --moteur {} --telecharger »",
            o.runtime, o.runtime
        ),
    })?;
    let format = Format::guess(&o.model);
    let hw = crate::hw::detect();
    let place = if o.cpu { Place::Cpu } else { Place::Gpu };
    let threads = o.threads.unwrap_or_else(|| crate::catalog::cpu_threads(&hw));
    let stem = o.model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let file_name = o.model.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let tuning_tag = if o.cpu {
        "proc".to_string()
    } else if o.tuning == Tuning::gentle() {
        "doux".into()
    } else if o.tuning == Tuning::raw() {
        "origine".into()
    } else {
        o.tuning.code().replace(['=', ';'], "-")
    };
    let tag = format!(
        "{stem}_{}{}_{tuning_tag}{}",
        place.code(),
        if o.low { "_low" } else { "" },
        o.bursts.map_or(String::new(), |b| format!("_r{}", b.tokens))
    );
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{} : {e}", o.out.display()))?;
    let log_path = o.out.join(format!("{tag}_serveur.log"));
    let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;

    crate::say!("Modèle : {} ({}), {}", o.model.display(), format.label(), place.label());
    crate::say!("Serveur : {}", server.display());
    if place == Place::Gpu {
        crate::say!("Réglages : {}", o.tuning.label());
    }
    crate::say!("Fils de calcul sur le processeur : {threads}");

    let gpu = main_gpu(&hw);
    let start_unix = crate::catalog::now_secs();
    let wow = crate::hw::wow_pids();
    if o.fps && wow.is_empty() && cfg!(windows) {
        crate::say!("ATTENTION : WoW n'est pas lancé ; le test en jeu n'a de sens que personnage en jeu.");
    }
    let abort = o.stop.clone().unwrap_or_default();
    let mon = Monitor::start(gpu.as_ref().map(|g| g.luid), wow.clone(), abort.clone());

    // 1. Référence : le jeu seul.
    let probe = if o.fps {
        match FpsProbe::start() {
            Ok(p) => Some(p),
            Err(e) => {
                crate::say!("Mesure des images par seconde impossible : {e}");
                None
            }
        }
    } else {
        None
    };
    let (mut base_fps, mut base_worst) = (Vec::new(), Vec::new());
    if let Some(p) = &probe {
        // Le test ne commence qu'une fois WoW au premier plan et les mesures de l'addon reçues.
        #[cfg(windows)]
        let in_front = o.from_game || crate::overlay_win::Overlay::bring_wow_to_front();
        #[cfg(not(windows))]
        let in_front = o.from_game;
        say(o, Status::Phase("En attente des mesures de l'addon".into()));
        crate::say!(
            "{} le test démarre dès que l'addon envoie ses mesures (personnage immobile, ne touche à rien ; un bip à la fin) ...",
            if in_front { "WoW mis au premier plan :" } else { "Passe dans WoW :" }
        );
        let t = Instant::now();
        let mut hinted = 0;
        while p.timed_between(t, Instant::now()).len() < 3 {
            let waited = t.elapsed().as_secs();
            if waited >= 20 * (hinted + 1) {
                hinted += 1;
                crate::say!(
                    "  toujours aucune mesure après {waited} s : dans WoW, tape /chevaliers perf on (le réglage est gardé depuis la 0.8.3) ; \
                     WoW doit être au premier plan, sans fenêtre par-dessus le coin en bas à droite de l'écran (bandeau de l'addon)"
                );
            }
            if t.elapsed() > Duration::from_secs(120) {
                return Err("aucune mesure de l'addon en 2 min : /chevaliers perf on dans WoW, WoW au premier plan, rien par-dessus le coin en bas à droite (bandeau de l'addon)".into());
            }
            if abort.load(Ordering::Relaxed) {
                return Err(format!("test {ABORTED}"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        crate::say!("Référence sans traduction : 15 s, ne touche plus à rien jusqu'à la fin ...");
        say(o, Status::Phase("Référence sans traduction : 15 s, ne bouge pas".into()));
        let a = Instant::now();
        while a.elapsed() < Duration::from_secs(15) {
            if abort.load(Ordering::Relaxed) {
                return Err(format!("test {ABORTED}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        (base_fps, base_worst) = phase_perf(probe.as_ref(), a, Instant::now());
    }
    let limit = crate::regul::hitch_limit(&base_worst.iter().map(|w| *w as f32).collect::<Vec<_>>());
    let screen = wow_screen();
    if let Some(v) = wow_flavor() {
        crate::say!("  Version du jeu : {v}");
    }
    let cap = fps_cap(&base_fps);
    if let Some(b) = median(&base_fps) {
        crate::say!(
            "  Référence : {b:.1} images/s ({} mesures) ; plus longue image {} ; à-coup toléré : {limit:.0} ms",
            base_fps.len(),
            if base_worst.is_empty() {
                "non mesurée (addon antérieur à la 0.8)".to_string()
            } else {
                format!("{:.0} ms (95 % des demi-secondes)", percentile(&base_worst, 0.95))
            }
        );
    }

    // 2. Mémoire vidéo : la carte ne doit pas saturer une fois le modèle chargé (0.14.2 : plus de
    // réserve de 1 Go, seuil de saturation de la carte).
    let size = std::fs::metadata(&o.model).map(|m| m.len()).unwrap_or(0);
    let (need, measured) = match crate::catalog::measured_footprint(&file_name) {
        Some(b) => (b, true),
        None => (crate::catalog::footprint_estimate(size), false),
    };
    if place == Place::Gpu {
        if let Some((g, free)) = vram_now() {
            let sat = saturation_now();
            crate::say!(
                "Mémoire vidéo libre sur {g} : {} ; le modèle en occupera {} ({}) ; seuil de saturation {}",
                gib(free),
                gib(need),
                if measured { "mesuré" } else { "estimation" },
                gib(sat)
            );
            if need + sat > free {
                let msg = format!(
                    "mémoire vidéo insuffisante pour ce modèle en ce moment ({} libres, {} + {} de seuil nécessaires). Essayer un modèle plus léger (--forcer pour passer outre, déconseillé)",
                    gib(free),
                    gib(need),
                    gib(sat)
                );
                if o.forcer {
                    crate::say!("ATTENTION : {msg}");
                } else {
                    return Err(msg);
                }
            }
        }
    }

    // 3. Chargement.
    mon.set_phase("chargement");
    say(o, Status::Phase("Chargement du modèle".into()));
    let t0 = Instant::now();
    let launch = crate::llama::Launch { exe: &server, model: &o.model, port: o.port, place, threads, tuning: &o.tuning };
    let child = crate::llama::spawn(&launch, log)?;
    mon.server_pid.store(child.id(), Ordering::Relaxed);
    let mut srv = Server(child);
    let mut stop: Option<(Verdict, String)> = None;
    loop {
        if let Some(r) = check_alive(&mon, &mut srv) {
            stop = Some(r);
            break;
        }
        if let Ok((200, _)) = http(o.port, "GET", "/health", None, Duration::from_secs(2)) {
            break;
        }
        if t0.elapsed() > Duration::from_secs(600) {
            return Err(format!("modèle toujours pas chargé après 10 min : voir {}", log_path.display()));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let load = t0.elapsed().as_secs_f64();
    let pid = srv.0.id();
    let vram = crate::hw::gpu_memory_of(pid);
    let ram = ram_of(&srv.0);
    if stop.is_none() {
        if let (Place::Gpu, Some(v)) = (place, vram) {
            crate::catalog::record_footprint(&file_name, v);
        }
        crate::say!(
            "Chargé en {load:.1} s - mémoire vidéo du serveur : {} - mémoire vive : {}",
            vram.map_or("inconnue".into(), gib),
            ram.map_or("inconnue".into(), gib)
        );
        say(o, Status::Note(format!("Chargé en {load:.1} s, mémoire vidéo {}", vram.map_or("inconnue".into(), gib))));
        if place == Place::Gpu {
            let p = crate::llama::apply_priority(pid, place, &o.tuning);
            crate::say!("Priorité sur la carte : {p}");
            say(o, Status::Note(format!("Priorité sur la carte : {p}")));
        }
        if place == Place::Gpu && !o.forcer {
            if let Some((_, free)) = vram_now() {
                if free < saturation_now() {
                    stop = Some((
                        Verdict::Memory,
                        format!("il ne reste que {} de mémoire vidéo libre une fois le modèle chargé", gib(free)),
                    ));
                }
            }
        }
    }

    // 4. Modèle chargé, au repos : la mémoire occupée gêne-t-elle déjà le jeu ?
    let (mut idle_fps, mut idle_worst) = (Vec::new(), Vec::new());
    if stop.is_none() && probe.is_some() {
        mon.set_phase("repos");
        crate::say!("Modèle chargé, au repos : 15 s ...");
        say(o, Status::Phase("Modèle chargé, au repos : 15 s".into()));
        let a = Instant::now();
        stop = watch_for(Duration::from_secs(15), &mon, &mut srv);
        (idle_fps, idle_worst) = phase_perf(probe.as_ref(), a, Instant::now());
    }

    // 5. Traductions, au rythme du chat en jeu.
    let cases = cases(o.court, o.low);
    if o.low {
        crate::say!("Version low : {} (30 paires), {} traductions", LOW.join(", "), cases.len());
    }
    let mut pace = o.pace.unwrap_or(if o.fps { 3.0 } else { 0.0 });
    let mut reg = (o.regulate && place == Place::Gpu && probe.is_some()).then(|| Regulator::new(limit));
    let bursts: Cell<Option<Bursts>> = Cell::new(o.bursts);
    if let Some(b) = o.bursts {
        crate::say!("Génération par {} (les messages longs sont aussi découpés en phrases)", b.label());
    }
    if stop.is_none() {
        // Échauffement (non compté).
        let (path, body) = request(format, &cases[0]);
        let _ = http(o.port, "POST", path, Some(&body), Duration::from_secs(300));
        mon.set_phase("traductions");
        say(o, Status::Phase(format!("Traductions : 0/{}", cases.len())));
        if pace > 0.0 {
            crate::say!("Une traduction toutes les {pace:.0} s au plus, comme un chat animé{}", if reg.is_some() { " ; régulateur actif" } else { "" });
        }
    }
    let file = o.out.join(format!("{tag}.tsv"));
    let mut tsv = format!(
        "# {stem} {} ({}) - {} - chargement {load:.1} s\n# origine\tcible\tsecondes\tjetons\tdéfaut\tmessage\ttraduction\n",
        place.label(),
        format.label(),
        if place == Place::Gpu { o.tuning.code() } else { format!("{threads} fils") }
    );
    let mut lat = Vec::new();
    let (mut tokens, mut gen_time, mut flagged, mut errors, mut consecutive) = (0u64, 0.0f64, 0usize, 0usize, 0usize);
    let mut windows: Vec<(Instant, Instant)> = Vec::new();
    let mut last_start: Option<Instant> = None;
    let mut burst_count = 0u32;
    let mut notes: Vec<String> = Vec::new();
    let t_run = Instant::now();
    let mut seen = t_run;
    let mut gap_warned = false;
    for (i, c) in cases.iter().enumerate() {
        if stop.is_some() {
            break;
        }
        // Attente du rythme, en surveillant le jeu et les à-coups.
        let next = last_start.map_or(t_run, |a| a + Duration::from_secs_f64(pace));
        while stop.is_none() && Instant::now() < next {
            stop = check_alive(&mon, &mut srv);
            if let Some(p) = &probe {
                let now = Instant::now();
                let quiet = p.timed_between(now - Duration::from_secs(3), now).is_empty();
                if quiet && !gap_warned && now.duration_since(t_run) > Duration::from_secs(3) {
                    crate::say!("  [mesures] plus de mesure de l'addon : WoW n'est plus au premier plan ? (reviens dans WoW)");
                    gap_warned = true;
                } else if !quiet {
                    gap_warned = false;
                }
            }
            if let (Some(r), Some(p), None) = (reg.as_mut(), probe.as_ref(), &stop) {
                {
                let before = notes.len();
                stop = regulate(r, p, &mut seen, &windows, None, pid, &mut pace, &bursts, &mut notes);
                for n in &notes[before..] {
                    say(o, Status::Note(n.trim_start_matches("[régulateur] ").to_string()));
                }
            }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if stop.is_some() {
            break;
        }
        // Mémoire vidéo : loin de la saturation (WoW peut grossir) ?
        if place == Place::Gpu && !o.forcer {
            if let (Some(g), Some(row)) = (&gpu, mon.rows().last()) {
                if let Some(u) = row.card_used {
                    let free = g.dedicated.saturating_sub(u);
                    if free < crate::gouverneur::saturation(g.dedicated) {
                        stop = Some((Verdict::Memory, format!("plus que {} de mémoire vidéo libre après {i} traductions", gib(free))));
                        break;
                    }
                }
            }
        }
        let t = Instant::now();
        last_start = Some(t);
        let r = translate_message(o.port, format, c, &bursts, &mon.busy, &mut windows, &mut burst_count);
        let secs = t.elapsed().as_secs_f64();
        let (text, ntok) = match r {
            Ok(x) => {
                consecutive = 0;
                x
            }
            Err(e) => {
                errors += 1;
                consecutive += 1;
                (format!("<erreur : {e}>"), None)
            }
        };
        let one_line = text.replace(['\n', '\t'], " ");
        let f = flag(c, &text);
        if f.is_some() {
            flagged += 1;
        }
        lat.push(secs);
        if let Some(n) = ntok {
            tokens += n;
            gen_time += secs;
        }
        crate::say!(
            "[{:>3}/{}] {}→{} {:>5.2} s{} : {}",
            i + 1,
            cases.len(),
            c.src,
            c.tgt,
            secs,
            f.map_or(String::new(), |f| format!(" [{f}]")),
            one_line
        );
        say(o, Status::Phase(format!("Traductions : {}/{}", i + 1, cases.len())));
        say(o, Status::Translation(format!("{}→{} {:.2} s : {}", c.src, c.tgt, secs, one_line).replace('.', ",")));
        tsv.push_str(&format!(
            "{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\n",
            c.src,
            c.tgt,
            secs,
            ntok.map_or(String::new(), |n| n.to_string()),
            f.unwrap_or(""),
            c.text,
            one_line
        ));
        if consecutive >= 3 {
            stop = Some((Verdict::Crash, "le serveur de traduction ne répond plus".into()));
        }
        if stop.is_none() {
            stop = check_alive(&mon, &mut srv);
        }
        if let (Some(r), Some(p), None) = (reg.as_mut(), probe.as_ref(), &stop) {
            {
                let before = notes.len();
                stop = regulate(r, p, &mut seen, &windows, None, pid, &mut pace, &bursts, &mut notes);
                for n in &notes[before..] {
                    say(o, Status::Note(n.trim_start_matches("[régulateur] ").to_string()));
                }
            }
        }
    }
    let t_end = Instant::now();
    let vram_after = crate::hw::gpu_memory_of(pid);
    if let (Place::Gpu, Some(v)) = (place, vram_after) {
        if vram.map_or(true, |b| v > b) {
            crate::catalog::record_footprint(&file_name, v);
        }
    }
    let ram_after = ram_of(&srv.0);
    mon.set_phase("fin");
    drop(srv);
    let aborted = stop.as_ref().is_some_and(|(_, w)| w == ABORTED);
    if let Some((_, why)) = &stop {
        crate::say!("Arrêt du test : {why}");
        say(o, Status::Note(format!("Arrêt du test : {why}")));
    }

    // 6. Réinitialisations du pilote graphique pendant le test (journaux de Windows).
    let mut resets = Vec::new();
    if cfg!(windows) {
        crate::say!("Lecture des journaux de Windows (10 s) ...");
        say(o, Status::Phase("Lecture des journaux de Windows (10 s)".into()));
        std::thread::sleep(Duration::from_secs(10));
        match crate::diag::events_since(start_unix) {
            Ok(ev) => resets = crate::diag::driver_resets(&ev),
            Err(e) => crate::say!("  journaux illisibles : {e}"),
        }
    }
    let rows = mon.rows();
    drop(mon);
    let run_perf = probe.as_ref().map(|p| p.timed_between(t_run, t_end)).unwrap_or_default();
    drop(probe);

    // --- Bilan ---------------------------------------------------------------------------
    let total = t_run.elapsed().as_secs_f64().min(t_end.duration_since(t_run).as_secs_f64());
    let summary = format!(
        "Résultat {stem} {} : {} traductions en {total:.0} s ; latence médiane {:.2} s, 95 % sous {:.2} s, maximum {:.2} s ; {:.0} jetons/s ; défauts visibles {flagged}, erreurs {errors} ; chargement {load:.1} s ; mémoire vidéo {} ; mémoire vive {}",
        place.label(),
        lat.len(),
        percentile(&lat, 0.5),
        percentile(&lat, 0.95),
        percentile(&lat, 1.0),
        if gen_time > 0.0 { tokens as f64 / gen_time } else { 0.0 },
        vram.max(vram_after).map_or("inconnue".into(), gib),
        ram.max(ram_after).map_or("inconnue".into(), gib),
    );
    let mut lines = vec![summary];

    // Images par seconde et à-coups.
    let busy_at = |ts: Instant| {
        let from = ts.checked_sub(Duration::from_millis(500)).unwrap_or(ts);
        windows.iter().any(|(a, b)| *a <= ts && *b >= from)
    };
    let run_fps: Vec<f64> = run_perf.iter().map(|(_, p)| p.fps as f64).collect();
    let busy_fps: Vec<f64> = run_perf.iter().filter(|(t, _)| busy_at(*t)).map(|(_, p)| p.fps as f64).collect();
    let busy_worst: Vec<f64> =
        run_perf.iter().filter(|(t, _)| busy_at(*t)).filter_map(|(_, p)| p.worst_ms.map(|w| w as f64)).collect();
    let base = median(&base_fps);
    let drop_idle = base.zip(median(&idle_fps)).map(|(b, v)| pct(b, v));
    let avg = (!run_fps.is_empty()).then(|| run_fps.iter().sum::<f64>() / run_fps.len() as f64);
    let drop_avg = base.zip(avg).map(|(b, v)| pct(b, v));
    let drop_busy = base.zip(median(&busy_fps)).map(|(b, v)| pct(b, v));
    let mut problems: Vec<String> = Vec::new();
    match base {
        Some(b) => {
            let mut l = format!("Images par seconde : référence {b:.1}");
            if let (Some(i), Some(d)) = (median(&idle_fps), drop_idle) {
                l += &format!(" ; modèle chargé au repos {i:.1} (baisse {:.1} %)", d * 100.0);
            }
            if let (Some(a), Some(d)) = (avg, drop_avg) {
                l += &format!(
                    " ; pendant le test{} : moyenne {a:.1} (baisse {:.1} %)",
                    if pace > 0.0 { format!(", une traduction toutes les {pace:.0} s au plus") } else { String::new() },
                    d * 100.0
                );
            }
            if let (Some(v), Some(d)) = (median(&busy_fps), drop_busy) {
                l += &format!(" ; pendant une traduction {v:.1} (baisse {:.1} %)", d * 100.0);
            }
            lines.push(l);
            if !busy_worst.is_empty() {
                lines.push(format!(
                    "À-coups : plus longue image {:.0} ms sans traduction, {} au repos, {:.0} ms pendant les traductions (95 % sous {:.0} ms) ; seuil {limit:.0} ms",
                    percentile(&base_worst, 1.0),
                    if idle_worst.is_empty() { "-".to_string() } else { format!("{:.0} ms", percentile(&idle_worst, 1.0)) },
                    percentile(&busy_worst, 1.0),
                    percentile(&busy_worst, 0.95)
                ));
                if percentile(&busy_worst, 0.95) > limit as f64 {
                    problems.push(format!("à-coups de {:.0} ms pendant les traductions (seuil {limit:.0} ms)", percentile(&busy_worst, 0.95)));
                }
            }
        }
        None if o.fps => lines.push("Images par seconde : pas de mesure (en jeu : /chevaliers perf on)".into()),
        None => lines.push("Images par seconde : non mesurées (test hors jeu ; option --fps pour le test en jeu)".into()),
    }
    // Conditions du test : la règle de choix des modèles en dépend.
    if let Some(b) = base {
        lines.push(format!(
            "Conditions : WoW {}, {b:.1} images/s sans traduction, {}",
            screen.map_or("(taille inconnue)".to_string(), |(w, h)| format!("en {w}x{h}")),
            match cap {
                Some(c) => format!("plafonné à {c:.0} (la carte a du temps libre : baisse sous-estimée pour un jeu sans plafond)"),
                None => "non plafonné (la carte est occupée par le jeu)".to_string(),
            }
        ));
    }
    // Mémoire vidéo pendant le test.
    if let Some(g) = &gpu {
        let during: Vec<&Row> = rows.iter().filter(|r| r.phase == "traductions" || r.phase == "repos").collect();
        let min_free = during.iter().filter_map(|r| r.card_used).map(|u| g.dedicated.saturating_sub(u)).min();
        let max_wow = rows.iter().filter_map(|r| r.wow).max();
        if let Some(f) = min_free {
            lines.push(format!(
                "Mémoire vidéo : libre au plus bas {} pendant le test{}",
                gib(f),
                max_wow.map_or(String::new(), |w| format!(" ; WoW jusqu'à {}", gib(w)))
            ));
        }
    }
    if cfg!(windows) {
        lines.push(if resets.is_empty() {
            "Pilote graphique : aucune réinitialisation pendant le test".into()
        } else {
            format!(
                "Pilote graphique : {} réinitialisation(s) pendant le test ({})",
                resets.len(),
                resets.iter().map(|(k, c)| format!("{k}, code {c}")).collect::<Vec<_>>().join(" ; ")
            )
        });
    }
    if let Some(b) = bursts.get() {
        lines.push(format!("Rafales : {burst_count} au total, réglage final : {}", b.label()));
    }
    if let Some(r) = &reg {
        lines.push(match r.peak {
            0 => "Régulateur : resté au réglage de départ".to_string(),
            n => format!("Régulateur : monté jusqu'au cran {n} ({})", notes.last().cloned().unwrap_or_default()),
        });
    }

    // Verdict.
    let med = percentile(&lat, 0.5);
    let max_lat = crate::catalog::MAX_LATENCY;
    if med > max_lat {
        problems.push(format!("latence médiane {med:.2} s (seuil {max_lat} s)"));
    }
    let limit_drop = crate::catalog::MAX_FPS_DROP;
    if let Some(d) = drop_idle.filter(|d| *d > limit_drop) {
        problems.push(format!("le modèle chargé ralentit déjà le jeu au repos (baisse {:.1} %)", d * 100.0));
    }
    // Capacité : la baisse suit le nombre de messages traduits par minute. Combien ce modèle en
    // traduit-il en gardant la baisse sous 5 % ? Il en faut 12 (un toutes les 5 s) ; au-delà,
    // la traduction espace les messages au lieu de ralentir le jeu.
    let run_secs = t_end.duration_since(t_run).as_secs_f64();
    let rate = if run_secs > 0.0 { lat.len() as f64 * 60.0 / run_secs } else { 0.0 };
    if let Some(d) = drop_avg.filter(|_| rate > 0.0) {
        let cap = if d <= 0.0005 { f64::INFINITY } else { rate * limit_drop / d };
        let need = crate::plan::DESIGN_RATE;
        lines.push(format!(
            "Capacité : {} messages par minute en gardant la baisse sous {:.0} % ({:.0} par minute pendant le test, {:.1} % de baisse) ; il en faut {need:.0}",
            if cap.is_finite() { format!("{:.0}", cap.min(999.0)) } else { "plus de 999".into() },
            limit_drop * 100.0,
            rate,
            d * 100.0
        ));
        if cap < need {
            problems.push(format!(
                "baisse moyenne d'images par seconde {:.1} % à {rate:.0} messages par minute : {:.0} par minute sous {:.0} % (il en faut {need:.0})",
                d * 100.0,
                cap,
                limit_drop * 100.0
            ));
        }
    }
    let (verdict, why) = if !resets.is_empty() {
        (Verdict::Crash, format!("pilote graphique réinitialisé {} fois", resets.len()))
    } else if let Some((v, w)) = &stop {
        (*v, w.clone())
    } else if !problems.is_empty() {
        (Verdict::Heavy, problems.join(", "))
    } else {
        (Verdict::Ok, String::new())
    };
    // Mesures couvrant au moins 70 % des traductions, sinon pas de verdict en jeu.
    let expected = run_secs / 0.5;
    let coverage = if expected > 0.0 { run_perf.len() as f64 / expected } else { 0.0 };
    let in_game = base.is_some() && coverage >= 0.7;
    if base.is_some() && !in_game {
        lines.push(format!(
            "Mesures de l'addon pendant {:.0} % des traductions seulement (WoW pas au premier plan) : pas de verdict en jeu, refaire le test",
            coverage * 100.0
        ));
    }
    let incomplete = base.is_some() && !in_game && verdict == Verdict::Heavy;
    let verdict_line = if aborted {
        "Test arrêté à la demande : rien n'est noté".to_string()
    } else if incomplete {
        // Mesures incomplètes : un « trop lourd » ne vaut pas verdict (WoW à l'arrière-plan
        // pendant une partie du test, par exemple pour répondre à une fenêtre).
        format!("Verdict : aucun, mesures en jeu incomplètes ({why}) : refaire ce test en restant dans WoW")
    } else {
        match verdict {
        Verdict::Ok if in_game => "Verdict en jeu : compatible".to_string(),
        Verdict::Ok if o.fps => "Verdict : aucun problème vu, mais mesure en jeu incomplète".to_string(),
        Verdict::Ok => "Verdict : latence dans le seuil ; test en jeu à faire (--fps)".to_string(),
        Verdict::Heavy => format!("Verdict en jeu : trop lourd pour ce PC avec ces réglages - {why}"),
        Verdict::Crash => format!("Verdict : a fait planter le jeu - {why}"),
        Verdict::Memory => format!("Verdict : mémoire vidéo insuffisante - {why}"),
        }
    };
    lines.push(verdict_line);
    // Résultat retenu pour ce PC : test en jeu, ou plantage quel que soit le test.
    let recorded = !aborted && (in_game || verdict == Verdict::Crash);
    if recorded {
        let mut detail = if why.is_empty() { String::new() } else { format!("{why} ; ") };
        if place == Place::Gpu {
            let final_tuning = reg.as_ref().map_or(o.tuning.clone(), |r| r.rung().tuning.clone());
            detail += &final_tuning.code();
        } else {
            detail += &format!("{threads} fils");
        }
        if pace > 0.0 {
            detail += &format!(";espacement={pace:.0}");
        }
        if !lat.is_empty() {
            detail += &format!(";latence={med:.2}");
        }
        if let Some(b) = bursts.get() {
            detail += &format!(";rafales={}/{}", b.tokens, b.pause_ms);
        }
        if let Some((w, h)) = screen {
            detail += &format!(";ecran={w}x{h}");
        }
        // Version du jeu testée (retail et Forever n'ont pas les mêmes résultats).
        if let Some(v) = wow_flavor() {
            detail += &format!(";version={v}");
        }
        if let Some(b) = base {
            detail += &format!(";ref={b:.1};plafond={}", cap.map_or("non".to_string(), |c| format!("{c:.0}")));
        }
        crate::catalog::record_trial(
            &file_name,
            place,
            &crate::catalog::Trial { verdict, when: start_unix, drop: drop_avg, detail },
        );
        lines.push(format!("Résultat noté pour ce PC ({})", crate::catalog::trials_file().display()));
    }

    // Chronologie : une ligne par seconde.
    let chrono = o.out.join(format!("{tag}_chronologie.tsv"));
    let mut ct = String::from("# secondes\tphase\tcarte_libre_go\tserveur_go\twow_go\timages_s\tplus_longue_image_ms\ttraduction\n");
    let all_perf = run_perf; // mesures des traductions ; les autres phases n'ont que la mémoire
    let g_ded = gpu.as_ref().map(|g| g.dedicated);
    let first = rows.first().map(|r| r.at);
    for r in &rows {
        let p = all_perf.iter().rev().find(|(t, _)| *t <= r.at).map(|(_, p)| *p);
        let gb = |v: Option<u64>| v.map_or(String::new(), |v| format!("{:.2}", v as f64 / GIB as f64));
        ct.push_str(&format!(
            "{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            first.map_or(0.0, |f| r.at.duration_since(f).as_secs_f64()),
            r.phase,
            gb(g_ded.zip(r.card_used).map(|(d, u)| d.saturating_sub(u))),
            gb(r.server),
            gb(r.wow),
            p.map_or(String::new(), |p| format!("{:.1}", p.fps)),
            p.and_then(|p| p.worst_ms).map_or(String::new(), |w| format!("{w:.0}")),
            if r.busy { "1" } else { "0" }
        ));
    }
    let _ = std::fs::write(&chrono, ct);

    for l in &lines {
        tsv.push_str(&format!("# {l}\n"));
    }
    std::fs::write(&file, tsv).map_err(|e| format!("{} : {e}", file.display()))?;
    crate::say!("");
    for l in &lines {
        crate::say!("{l}");
    }
    crate::say!("Traductions : {}", file.display());
    crate::say!("Chronologie (mémoire vidéo, images par seconde, chaque seconde) : {}", chrono.display());
    say(o, Status::Done { lines: lines.clone(), verdict, recorded, aborted });
    if o.fps && !o.from_game {
        // Fin du test en jeu : bip, et la console revient devant si Windows le permet.
        print!("\x07");
        #[cfg(windows)]
        crate::overlay_win::Overlay::bring_console_to_front();
    }
    Ok(errors == 0 && verdict != Verdict::Crash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_covers_every_language() {
        let all = cases(false, false);
        assert_eq!(all.len(), 240);
        for (code, _, _) in LANGS {
            assert!(all.iter().any(|c| c.tgt == *code), "aucune traduction vers {code}");
        }
        assert!(all.iter().all(|c| c.src != c.tgt));
        assert!(all.iter().all(|c| LANGS.iter().any(|l| l.0 == c.tgt)), "code de langue inconnu");
        // Paires sans l'anglais : la majorité.
        assert!(all.iter().filter(|c| c.src != "en" && c.tgt != "en").count() > 180);
        assert_eq!(cases(true, false).len(), 48);
        assert!(all.iter().all(|c| c.src != "it"), "messages italiens réservés à la version low");
    }

    #[test]
    fn low_version_covers_the_30_pairs() {
        let low = cases(false, true);
        assert_eq!(low.len(), 150);
        for s in LOW {
            for t in LOW.iter().filter(|t| *t != s) {
                let n = low.iter().filter(|c| c.src == *s && c.tgt == *t).count();
                assert_eq!(n, 5, "{s}→{t}");
            }
        }
        assert!(low.iter().all(|c| LOW.contains(&c.src.as_str()) && LOW.contains(&c.tgt.as_str())));
        assert!(low.iter().any(|c| c.text.starts_with("torno tra 5 min")));
        assert!(low.iter().any(|c| c.text == "brb 5 min, je mange"));
        let short = cases(true, true);
        assert_eq!(short.len(), 60);
        assert!(short.iter().filter(|c| c.src == "it").count() == 10);
    }

    #[test]
    fn requests_and_replies() {
        let c = Case { src: "de".into(), tgt: "pl".into(), text: "Danke!".into() };
        let (p, b) = request(Format::TranslateGemma, &c);
        assert_eq!(p, "/completion");
        let prompt = b["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("<start_of_turn>user\nYou are a professional German (de) to Polish (pl) translator."));
        assert!(prompt.ends_with("Danke!<end_of_turn>\n<start_of_turn>model\n"));
        let (p, b) = request(Format::Salamandra, &c);
        assert_eq!(p, "/v1/chat/completions");
        assert_eq!(
            b["messages"][0]["content"],
            "Translate the following text from German into Polish.\nGerman: Danke!\nPolish:"
        );
        let v: Value = serde_json::from_str(r#"{"content":" Dziękuję! ","timings":{"predicted_n":4}}"#).unwrap();
        assert_eq!(parse_reply(&v), ("Dziękuję!".into(), Some(4)));
        let v: Value = serde_json::from_str(r#"{"choices":[{"message":{"content":"Dzięki!"}}],"usage":{"completion_tokens":3}}"#).unwrap();
        assert_eq!(parse_reply(&v), ("Dzięki!".into(), Some(3)));
        assert_eq!(Format::guess(Path::new("x/translategemma-4b-it.Q4_K_M.gguf")), Format::TranslateGemma);
        assert_eq!(Format::guess(Path::new("salamandraTA_7B_v3_q4_k_m.gguf")), Format::Salamandra);
        assert_eq!(Format::guess(Path::new("salamandrata_2b_inst_q4.gguf")), Format::Salamandra);
        assert_eq!(Format::guess(Path::new("EuroLLM-1.7B-Instruct.Q4_K_M.gguf")), Format::EuroLlm);
        let (_, b) = request(Format::EuroLlm, &c);
        assert_eq!(
            b["messages"][0]["content"],
            "Translate the following German source text to Polish:\nGerman: Danke!\nPolish: "
        );
    }

    #[test]
    fn fps_cap_detection() {
        assert_eq!(fps_cap(&[75.0; 30]), Some(75.0));
        assert_eq!(fps_cap(&[41.8, 41.7, 42.0, 41.9, 41.6, 41.8, 41.7]), None);
        let jittery: Vec<f64> = (0..30).map(|i| 60.0 + (i % 5) as f64 * 0.8 - 1.6).collect();
        assert_eq!(fps_cap(&jittery), None);
        assert_eq!(fps_cap(&[60.0, 59.9, 60.0]), None);
    }

    #[test]
    fn language_label_removed() {
        let c = Case { src: "de".into(), tgt: "pl".into(), text: "Danke!".into() };
        assert_eq!(clean(&c, " Polish: Dzięki! "), "Dzięki!");
        assert_eq!(clean(&c, "Dzięki!"), "Dzięki!");
        // Étiquettes réelles d'EuroLLM 1.7B (09/11/2026).
        for (raw, want) in [
            ("Translation: Hello everyone, is anyone up for the Death Mine?", "Hello everyone, is anyone up for the Death Mine?"),
            ("Francés: Saludos a todos", "Saludos a todos"),
            ("Französisch: LF-Tank und Heil für Magma Heart", "LF-Tank und Heil für Magma Heart"),
            ("Русский: Привет всем", "Привет всем"),
            ("Traduction du texte russe original en français : Tout le monde, qui ira ?", "Tout le monde, qui ira ?"),
            ("Traducción del texto en ruso a español: ¡Hola a todos!", "¡Hola a todos!"),
            ("Tradotto dalla lingua russa a italiano: Saluti a tutti", "Saluti a tutti"),
            ("Traduction de la phrase en français : « Salut à tous, quelqu'un ?", "Salut à tous, quelqu'un ?"),
            ("Italian: Translation: Ciao", "Ciao"),
            // Pas des étiquettes : à garder.
            ("LFM Molten Core, besoin de 1 tank : urgent", "LFM Molten Core, besoin de 1 tank : urgent"),
            ("WTS [Thunderfury]: 500g", "WTS [Thunderfury]: 500g"),
            ("« Salut » dit-il", "« Salut » dit-il"),
        ] {
            assert_eq!(clean(&c, raw), want, "{raw}");
        }
    }

    #[test]
    fn flags() {
        let c = Case { src: "fr".into(), tgt: "en".into(), text: "gg tout le monde".into() };
        assert_eq!(flag(&c, ""), Some("vide"));
        assert_eq!(flag(&c, "GG  tout le monde"), Some("recopie"));
        assert_eq!(flag(&c, "Here is the translation: gg all"), Some("préambule"));
        assert_eq!(flag(&c, "gg everyone"), None);
        assert_eq!(flag(&c, "gg everyone\n(note)"), Some("plusieurs lignes"));
    }

    #[test]
    fn chunked_bodies() {
        assert_eq!(dechunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), b"Wikipedia");
    }

    /// Faux serveur : vérifie l'aller-retour HTTP sans llama.cpp.
    #[test]
    fn http_roundtrip() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            let mut buf = [0u8; 4096];
            while !req.contains("{\"x\":1}") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                req.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n").unwrap();
            req
        });
        let (code, body) = http(port, "POST", "/completion", Some(&json!({"x": 1})), Duration::from_secs(5)).unwrap();
        assert_eq!((code, body.as_str()), (200, "{\"a\":1}"));
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /completion HTTP/1.1") && req.contains("{\"x\":1}"));
    }
}
