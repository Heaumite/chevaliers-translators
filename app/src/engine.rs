#![cfg_attr(not(windows), allow(dead_code))]
//! Traduction en usage réel : le serveur llama.cpp tourne sur la première étape de la chaîne de
//! repli (estimation, `plan.rs`) dont le modèle est installé, et passe à l'étape suivante si
//! celle-ci échoue en jeu — sans test chez le joueur :
//!   - plantage du serveur ou manque de mémoire : noté (essais.tsv), étape suivante permise ;
//!   - trop lent (latence médiane des 10 derniers messages au-delà du seuil) : noté, prochaine
//!     étape plus rapide.
//!
//! Pour que la traduction paraisse instantanée :
//!   - jusqu'à 4 messages sont traduits à la fois : le serveur les calcule ensemble, en relisant
//!     le modèle une seule fois par jeton pour tous (la charge sur la carte, qui suit ces relectures,
//!     est celle d'un seul message). Les messages reçus partent par vagues : au plus une vague toutes
//!     les 60 / capacité secondes, pour que la baisse d'images par seconde reste sous 5 % ;
//!   - un message reçu qui ne serait pas prêt avant que sa ligne doive s'afficher (4 s) n'est pas
//!     traduit : il s'affiche tel quel ;
//!   - les demandes du joueur (Maj+Entrée, puis brouillons en cours de frappe) passent devant ;
//!   - les phrases déjà traduites sont gardées sur le disque (`phrases.tsv`) et resservent, sans
//!     calcul, d'une partie à l'autre (annonces du commerce, formules de politesse). Les liens
//!     d'objets y sont des repères : « WTS [1] pst » sert quel que soit l'objet.
//!
//! Liens, icônes, noms du jeu et jargon sont protégés (`glossary.rs`).

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::bench::{self, Case, Format};
use crate::burst::{self, Bursts};
use crate::catalog::{self, Place, Trial, Verdict};
use crate::glossary::{self, Protected};
use crate::plan::{self, Candidate, Failure, Plan};

/// Ordre de passage : l'envoi du joueur, puis son brouillon, puis les messages reçus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Send,
    Draft,
    Received,
}

/// Une traduction demandée.
#[derive(Clone, Debug)]
pub struct Ask {
    pub text: String,
    pub src: &'static str,
    pub tgt: &'static str,
    pub priority: Priority,
    /// Brouillons d'une même saisie : un brouillon plus récent remplace les anciens en attente.
    pub group: Option<u64>,
}

/// Une traduction à faire.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub ask: Ask,
    pub queued: Instant,
}

#[derive(Debug)]
pub enum Event {
    Done { id: u64, result: Result<String, String>, secs: f64, cached: bool },
    /// Modèle actif, repli, erreur de lancement : à afficher.
    Status(String),
    /// Modèle supérieur à proposer au joueur (mesures en jeu, 0.14).
    Propose(crate::conseil::Proposal),
}

/// Ce que la boucle du jeu attend d'un traducteur (le vrai, ou un faux pour les tests).
pub trait Translator {
    fn submit(&mut self, ask: Ask) -> u64;
    fn poll(&mut self) -> Vec<Event>;
}

pub struct Config {
    /// Dossier de la version de WoW (réglages graphiques pour l'estimation).
    pub wow: Option<PathBuf>,
    pub models: PathBuf,
    pub port: u16,
    pub log: PathBuf,
    /// Mémoire des phrases (None : en mémoire seulement).
    pub phrases: Option<PathBuf>,
}

pub struct Engine {
    tx: Sender<Job>,
    rx: Receiver<Event>,
    next: u64,
}

impl Engine {
    pub fn start(cfg: Config) -> Engine {
        let (jtx, jrx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        std::thread::spawn(move || worker(cfg, jrx, etx));
        Engine { tx: jtx, rx: erx, next: 1 }
    }
}

impl Translator for Engine {
    fn submit(&mut self, ask: Ask) -> u64 {
        let id = self.next;
        self.next += 1;
        let _ = self.tx.send(Job { id, ask, queued: Instant::now() });
        id
    }
    fn poll(&mut self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

// --- Contexte du joueur (addon 0.13.3) ---------------------------------------------------------

/// Contexte de chaque jeu ouvert (0.15 : plusieurs à la fois), par numéro de jeu.
static CONTEXTS: std::sync::Mutex<Vec<(usize, u8, u8)>> = std::sync::Mutex::new(Vec::new());
pub const CTX_COMBAT: u8 = 1;
pub const CTX_INSTANCE: u8 = 2;
pub const CTX_RAID: u8 = 4;
pub const CTX_AFK: u8 = 8;
pub const CTX_LOCKED: u8 = 16;

/// Contexte reçu de l'addon : combat, instance, raid, absence, chat verrouillé. Reçu une fois :
/// le joueur est en jeu (le premier modèle est choisi d'après la mémoire vidéo d'alors).
pub fn set_context(c: u8) {
    set_context_of(0, c);
}

/// Contexte d'un des jeux ouverts (0.15) : le moteur voit l'union de tous (un jeu en combat
/// suffit à retenir les bascules de modèle).
pub fn set_context_of(game: usize, c: u8) {
    if let Ok(mut v) = CONTEXTS.lock() {
        match v.iter_mut().find(|e| e.0 == game) {
            Some(e) => e.1 = c,
            None => v.push((game, c, 0)),
        }
    }
    IN_WORLD.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Jeu fermé : son contexte ne compte plus.
pub fn forget_game(game: usize) {
    if let Ok(mut v) = CONTEXTS.lock() {
        v.retain(|e| e.0 != game);
    }
}

static IN_WORLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Réglages graphiques de WoW (addon 0.14.2) : numéro changé par l'addon à chaque réglage
/// graphique modifié dans le jeu (0 : connexion ou /reload).
pub fn set_gfx(g: u8) {
    set_gfx_of(0, g);
}

/// Réglages graphiques d'un des jeux ouverts : le numéro vu par le moteur change dès que l'un
/// d'eux change.
pub fn set_gfx_of(game: usize, g: u8) {
    if let Ok(mut v) = CONTEXTS.lock() {
        match v.iter_mut().find(|e| e.0 == game) {
            Some(e) => e.2 = g,
            None => v.push((game, 0, g)),
        }
    }
}

fn gfx() -> u8 {
    CONTEXTS.lock().map(|v| v.iter().fold(0u8, |a, e| a.wrapping_add(e.2))).unwrap_or(0)
}

pub fn context() -> u8 {
    CONTEXTS.lock().map(|v| v.iter().fold(0u8, |a, e| a | e.1)).unwrap_or(0)
}

// --- Commandes (0.14) : téléchargements finis, propositions acceptées ou écartées --------------

/// Commande pour le moteur, traitée hors combat (sauf `Refuse`).
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// Modèles installés ou mesures changés : chaîne recalculée, le meilleur modèle installé pris.
    Replan,
    /// Proposition acceptée (`/chevaliers monter`).
    Accept,
    /// Proposition écartée (`/chevaliers non`).
    Refuse,
    /// Modèle choisi par le joueur, maintenant installé : essayé.
    Prefer(&'static str, Place),
}

static MAILBOX: std::sync::Mutex<Vec<Control>> = std::sync::Mutex::new(Vec::new());

pub fn post(c: Control) {
    if let Ok(mut m) = MAILBOX.lock() {
        if !m.contains(&c) {
            m.push(c);
        }
    }
}

/// Commandes à traiter maintenant : toutes hors combat, seulement les refus en combat.
fn take_controls(combat: bool) -> Vec<Control> {
    let Ok(mut m) = MAILBOX.lock() else { return Vec::new() };
    if !combat {
        return std::mem::take(&mut *m);
    }
    let (now, later): (Vec<Control>, Vec<Control>) = m.drain(..).partition(|c| *c == Control::Refuse);
    *m = later;
    now
}

/// En combat (ou chat verrouillé) : ni bascule de modèle, ni panneau sur l'écran.
pub fn in_combat() -> bool {
    context() & (CTX_COMBAT | CTX_LOCKED) != 0
}

/// Relevé de la mémoire vidéo pendant le jeu (module `gouverneur`) ; plus souvent près de la
/// saturation.
const VRAM_CHECK: Duration = Duration::from_secs(5);
const VRAM_CHECK_NEAR: Duration = Duration::from_secs(2);

/// Traductions menées en même temps (emplacements du serveur, `llama::SLOTS`).
pub const PARALLEL: usize = crate::llama::SLOTS as usize;
/// Temps laissé à un message reçu pour être traduit (0.12.4 : 60 s ; avant, 4 s). Sa ligne
/// s'affiche telle quelle au bout de `mirror::HOLD_MAX` au plus, et la traduction la remplace à
/// son arrivée : tout message dans une autre langue que celle du client est traduit.
pub const RECEIVED_DEADLINE: Duration = Duration::from_secs(60);

// --- Mémoire des phrases --------------------------------------------------------------------------

type Key = (String, String, String);

/// Phrases déjà traduites, par (origine, cible, phrase avec ses repères). Chaque traduction garde
/// la qualité du modèle qui l'a faite : un modèle meilleur ne réutilise pas celle d'un moins bon.
pub struct Memory {
    map: HashMap<Key, (String, u8)>,
    order: VecDeque<Key>,
    cap: usize,
    file: Option<PathBuf>,
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Memory {
    pub fn new(cap: usize) -> Memory {
        Memory { map: HashMap::new(), order: VecDeque::new(), cap, file: None }
    }

    /// Relit le fichier (les plus récentes l'emportent) ; s'il a beaucoup grossi, il est réécrit.
    pub fn load(path: &Path, cap: usize) -> Memory {
        let mut m = Memory::new(cap);
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut lines = 0;
        for l in text.lines() {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            if f.len() == 5 {
                lines += 1;
                m.insert(f[0], f[1], &unesc(f[3]), unesc(f[4]), f[2].parse().unwrap_or(1));
            }
        }
        if lines > cap * 2 {
            let mut out = String::new();
            for k in &m.order {
                let (t, q) = &m.map[k];
                out.push_str(&format!("{}\t{}\t{q}\t{}\t{}\n", k.0, k.1, esc(&k.2), esc(t)));
            }
            let _ = std::fs::write(path, out);
        }
        m.file = Some(path.to_path_buf());
        m
    }

    fn key(piece: &str, src: &str, tgt: &str) -> Key {
        (src.to_string(), tgt.to_string(), piece.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    fn insert(&mut self, src: &str, tgt: &str, piece: &str, out: String, quality: u8) {
        let k = Self::key(piece, src, tgt);
        if self.map.insert(k.clone(), (out, quality)).is_none() {
            self.order.push_back(k);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }

    /// Traduction gardée, faite par un modèle au moins aussi bon que `quality`.
    pub fn get(&self, piece: &str, src: &str, tgt: &str, quality: u8) -> Option<&str> {
        self.map.get(&Self::key(piece, src, tgt)).filter(|(_, q)| *q >= quality).map(|(t, _)| t.as_str())
    }

    pub fn put(&mut self, piece: &str, src: &str, tgt: &str, out: &str, quality: u8) {
        self.insert(src, tgt, piece, out.to_string(), quality);
        if let Some(f) = &self.file {
            if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
                let k = Self::key(piece, src, tgt);
                let _ = writeln!(fh, "{src}\t{tgt}\t{quality}\t{}\t{}", esc(&k.2), esc(out));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Phrases d'un message (une traduction et une entrée de mémoire chacune) : coupé aux fins de
/// phrase, et les morceaux trop longs aux virgules.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        cur.push(*c);
        let end = matches!(c, '.' | '!' | '?' | '…') && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if end && cur.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
            out.push(cur.trim().to_string());
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        match out.last_mut() {
            // Reste sans lettres (« :) ») : avec la phrase d'avant.
            Some(last) if !cur.chars().any(char::is_alphabetic) => {
                last.push(' ');
                last.push_str(cur.trim());
            }
            _ => out.push(cur.trim().to_string()),
        }
    }
    out.into_iter().flat_map(|s| burst::split_sentences(&s, 160)).collect()
}

// --- File d'attente --------------------------------------------------------------------------------

/// Prochaines demandes à lancer, dans l'ordre : celles du joueur dès qu'un emplacement est libre ;
/// les messages reçus par vague (tous les emplacements libres d'un coup), une vague au plus toutes
/// les `gap` secondes.
pub fn pick(queue: &VecDeque<Job>, free: usize, wave_ok: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..queue.len()).collect();
    idx.sort_by_key(|&i| (queue[i].ask.priority, queue[i].queued));
    let mut out = Vec::new();
    for i in idx {
        if out.len() >= free {
            break;
        }
        if queue[i].ask.priority == Priority::Received && !wave_ok {
            continue;
        }
        out.push(i);
    }
    out
}

/// Messages reçus qui ne seraient plus prêts à temps (leur ligne s'affiche telle quelle).
#[cfg(test)]
pub fn too_late(job: &Job, now: Instant, expected: Duration) -> bool {
    job.ask.priority == Priority::Received && now + expected > job.queued + RECEIVED_DEADLINE
}

/// Messages reçus qui ne seraient pas prêts à temps, compte tenu de ceux qui passent avant eux
/// (vagues de `PARALLEL` au plus, une toutes les `max(expected, gap)`) : rendus tout de suite
/// (leur ligne s'affiche telle quelle) plutôt qu'au bout de l'attente (0.12.3, chat très actif).
/// `wave_busy` : une vague de messages reçus est en cours.
pub fn overdue(
    queue: &VecDeque<Job>,
    now: Instant,
    expected: Duration,
    next_wave: Instant,
    wave_busy: bool,
    gap: Duration,
) -> Vec<usize> {
    let period = expected.max(gap);
    let mut base = now.max(next_wave);
    if wave_busy {
        base = base.max(now + expected / 2);
    }
    let mut order: Vec<usize> = (0..queue.len()).filter(|&i| queue[i].ask.priority == Priority::Received).collect();
    order.sort_by_key(|&i| queue[i].queued);
    let mut ahead = 0usize;
    let mut out = Vec::new();
    for i in order {
        let finish = base + period * (ahead / PARALLEL) as u32 + expected;
        if finish > queue[i].queued + RECEIVED_DEADLINE {
            out.push(i);
        } else {
            ahead += 1;
        }
    }
    out.sort_unstable();
    out
}

// --- Traduction ------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Server {
    port: u16,
    format: Format,
    bursts: Option<Bursts>,
}

struct Running {
    child: Child,
    srv: Server,
    pos: usize,
    gap: f64,
    quality: u8,
    /// Modèle et place (« X sur la carte graphique ») : retrouvé dans une chaîne recalculée.
    label: Option<String>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn translate_piece(s: Server, piece: &str, src: &str, tgt: &str) -> Result<String, String> {
    let case = Case { src: src.to_string(), tgt: tgt.to_string(), text: piece.to_string() };
    let (path, body) = bench::request(s.format, &case);
    let out = match s.bursts {
        Some(b) => {
            let (prompt, params) = burst::raw_prompt(s.port, path, &body)?;
            burst::generate(s.port, &prompt, &params, 200, &|| b, None, &mut |_, _| {})?.text
        }
        None => {
            let (code, b) = bench::http(s.port, "POST", path, Some(&body), Duration::from_secs(60))?;
            if code != 200 {
                return Err(format!("HTTP {code}"));
            }
            let v: Value = serde_json::from_str(&b).map_err(|e| format!("réponse illisible : {e}"))?;
            bench::parse_reply(&v).0
        }
    };
    Ok(bench::clean(&case, out.trim()).replace('\n', " "))
}

/// Message en cours : phrases, et leurs traductions (gardées ou à faire).
struct Work {
    job: Job,
    prot: Protected,
    pieces: Vec<String>,
    outs: Vec<Option<String>>,
    started: Instant,
}

impl Work {
    fn new(job: Job, mem: &Memory, quality: u8) -> Work {
        let prot = glossary::protect(&job.ask.text, job.ask.src, job.ask.tgt);
        let pieces = if prot.text.chars().filter(|c| c.is_alphabetic()).count() < 2 { Vec::new() } else { sentences(&prot.text) };
        let outs = pieces.iter().map(|p| mem.get(p, job.ask.src, job.ask.tgt, quality).map(String::from)).collect();
        Work { job, prot, pieces, outs, started: Instant::now() }
    }

    fn missing(&self) -> Vec<String> {
        self.pieces.iter().zip(&self.outs).filter(|(_, o)| o.is_none()).map(|(p, _)| p.clone()).collect()
    }

    /// Texte final, une fois toutes les phrases traduites.
    fn text(&self) -> String {
        if self.pieces.is_empty() {
            return glossary::restore(&self.prot.text, &self.prot);
        }
        let joined: Vec<&str> = self.outs.iter().map(|o| o.as_deref().unwrap_or("")).collect();
        glossary::restore(&joined.join(" "), &self.prot)
    }
}

// --- Serveur ---------------------------------------------------------------------------------------

fn server_exe(place: Place) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let dir = catalog::runtime_dir();
    // Le moteur Vulkan sert aussi sur le processeur (--device none) ; à défaut, le moteur processeur.
    let order: &[&str] = if place == Place::Gpu { &["vulkan"] } else { &["vulkan", "cpu"] };
    order.iter().find_map(|r| catalog::runtime_build(&dir, r).map(|d| d.join(exe)).filter(|p| p.is_file()))
}

fn launch(cfg: &Config, plan: &Plan, pos: usize) -> Result<Running, (Failure, String)> {
    let c = plan.step(pos).ok_or((Failure::Crash, "étape inconnue".to_string()))?;
    let model = cfg.models.join(c.model.file);
    let exe = server_exe(c.place).ok_or((Failure::Crash, "moteur llama.cpp absent (installation --telecharger)".to_string()))?;
    let mut tuning = crate::llama::Tuning::gentle();
    // Cache 8 bits déjà refusé par ce moteur (0.15) : plus essayé.
    let refused = catalog::runtime_dir().join("cache-8bits-refuse.txt");
    let exe_key = exe.to_string_lossy().to_lowercase();
    if std::fs::read_to_string(&refused).is_ok_and(|t| t.lines().any(|l| l.trim() == exe_key)) {
        tuning.kv_cache_8bit = false;
    }
    match start_server(cfg, &exe, &model, c, plan, &tuning, pos) {
        // Moteur qui refuse le cache 8 bits (attention « flash » indisponible) : relancé sans, et
        // retenu pour les prochaines fois. Ce n'est pas un échec du modèle.
        Err((Failure::Memory, why)) if tuning.kv_cache_8bit && crate::llama::kv_refused(&std::fs::read_to_string(&cfg.log).unwrap_or_default()) => {
            crate::say!("[traduction] cache 8 bits refusé par ce moteur ({why}) : relance avec le cache ordinaire");
            let mut list = std::fs::read_to_string(&refused).unwrap_or_default();
            list.push_str(&exe_key);
            list.push('\n');
            let _ = std::fs::write(&refused, list);
            tuning.kv_cache_8bit = false;
            start_server(cfg, &exe, &model, c, plan, &tuning, pos)
        }
        r => r,
    }
}

fn start_server(
    cfg: &Config,
    exe: &Path,
    model: &Path,
    c: &Candidate,
    plan: &Plan,
    tuning: &crate::llama::Tuning,
    pos: usize,
) -> Result<Running, (Failure, String)> {
    let log = std::fs::File::create(&cfg.log).map_err(|e| (Failure::Crash, format!("{} : {e}", cfg.log.display())))?;
    let l = crate::llama::Launch { exe, model, port: cfg.port, place: c.place, threads: plan.threads, tuning };
    let child = crate::llama::spawn(&l, log).map_err(|e| (Failure::Crash, e))?;
    let mut r = Running {
        child,
        srv: Server { port: cfg.port, format: Format::guess(model), bursts: (c.place == Place::Gpu).then(Bursts::gentle) },
        pos,
        gap: c.min_gap(),
        quality: c.model.quality,
        label: Some(c.label()),
    };
    let t0 = Instant::now();
    loop {
        if let Ok(Some(st)) = r.child.try_wait() {
            // Arrêt pendant le chargement : presque toujours la mémoire.
            return Err((Failure::Memory, format!("le serveur s'est arrêté pendant le chargement ({st})")));
        }
        if let Ok((200, _)) = bench::http(cfg.port, "GET", "/health", None, Duration::from_secs(2)) {
            break;
        }
        if t0.elapsed() > Duration::from_secs(300) {
            return Err((Failure::Crash, "modèle toujours pas chargé après 5 min".into()));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    if c.place == Place::Gpu {
        let _ = crate::llama::apply_priority(r.child.id(), c.place, tuning);
        // Carte saturée une fois le modèle chargé (0.14.2 : seuil de saturation, plus de réserve).
        if let Some((_, free)) = bench::vram_now() {
            if free < bench::saturation_now() {
                return Err((Failure::Memory, format!("plus que {} de mémoire vidéo libre une fois le modèle chargé", crate::hw::gib(free))));
            }
        }
    }
    Ok(r)
}

/// Note l'échec pour ce PC : la chaîne du prochain lancement en tient compte.
fn note(c: &Candidate, f: Failure, detail: &str) {
    let mut screen = bench::wow_screen().map_or(String::new(), |(w, h)| format!(";ecran={w}x{h}"));
    // Version du jeu (0.12.6) : un échec sur retail ne vaut pas pour Forever.
    if let Some(v) = bench::wow_flavor() {
        screen += &format!(";version={v}");
    }
    let verdict = match f {
        Failure::Crash => Verdict::Crash,
        Failure::Memory => Verdict::Memory,
        Failure::Heavy | Failure::Slow => Verdict::Heavy,
    };
    catalog::record_trial(
        c.model.file,
        c.place,
        &Trial { verdict, when: catalog::now_secs(), drop: None, detail: format!("en jeu : {detail}{screen}") },
    );
}

fn median(v: &VecDeque<f64>) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().collect();
    s.sort_by(f64::total_cmp);
    s.get(s.len() / 2).copied().unwrap_or(0.0)
}

fn step_label(plan: &Plan, pos: usize) -> String {
    let c = plan.step(pos).unwrap();
    let gap = c.min_gap();
    format!(
        "{} (étape {} sur {} de la chaîne de repli{})",
        c.label(),
        pos + 1,
        plan.chain.len(),
        if gap > 3.0 { format!(", messages reçus par vagues de {PARALLEL} au plus toutes les {gap:.1} s").replace('.', ",") } else { String::new() }
    )
}

/// Le modèle choisi par le joueur pour ces conditions a échoué : choix oublié, la chaîne de la
/// règle reprend (la proposition reviendra, avec l'échec dans son avertissement).
fn drop_choice(key: &str, c: &Candidate) {
    if crate::conseil::choice(key).is_some_and(|(m, p)| m.id == c.model.id && p == c.place) {
        crate::conseil::set_choice(key, None);
    }
}

/// Chaîne de ce PC, avec le modèle choisi par le joueur en tête.
fn plan_with_choice(cfg: &Config, key: Option<&str>) -> (Plan, String) {
    let hw = crate::hw::detect();
    let mut plan = plan::for_this_pc(&hw, cfg.wow.as_deref());
    let k = key.map(String::from).unwrap_or_else(|| crate::mesures::key(plan.conditions.flavor.as_deref(), plan.conditions.size));
    crate::conseil::apply_choice(&mut plan, crate::conseil::choice(&k));
    (plan, k)
}

/// La chaîne vue par le gouverneur de la mémoire vidéo.
fn gov_steps(plan: &Plan, installed: &dyn Fn(&Candidate) -> bool) -> Vec<crate::gouverneur::Step> {
    (0..plan.chain.len())
        .filter_map(|p| plan.step(p))
        .map(|c| crate::gouverneur::Step { label: c.label(), gpu: c.place == Place::Gpu, footprint: c.footprint, usable: installed(c) })
        .collect()
}

/// Place pour le modèle à la première installation (aucune mesure dans ces conditions) : la carte,
/// moins les autres programmes, moins WoW compté 7 Go (règle de référence).
pub fn first_install_room(dedicated: u64, others: u64) -> u64 {
    dedicated.saturating_sub(others).saturating_sub(catalog::WOW_VRAM_REFERENCE)
}

/// Mémoire vidéo libre en comptant celle du modèle chargé (s'il est sur la carte).
fn available(plan: &Plan, running: Option<&Running>, free: u64) -> u64 {
    free + running.and_then(|r| plan.step(r.pos)).filter(|c| c.place == Place::Gpu).map_or(0, |c| c.footprint)
}

fn worker(cfg: Config, jobs: Receiver<Job>, ev: Sender<Event>) {
    let (mut plan, key) = plan_with_choice(&cfg, None);
    let models = cfg.models.clone();
    let installed = move |c: &Candidate| catalog::installed_in(&models, c.model);
    let models2 = cfg.models.clone();
    let installed_model = move |m: &catalog::Model| catalog::installed_in(&models2, m);
    // Mémoire vidéo gérée en jeu (0.14.2) : premier modèle choisi une fois le joueur en jeu, le
    // meilleur qui tient alors ; descente près de la saturation, remontée quand la place revient.
    let mut gov = crate::gouverneur::Governor::new(plan.card.as_ref().map_or(0, |c| c.dedicated));
    IN_WORLD.store(false, std::sync::atomic::Ordering::Relaxed);
    let started = Instant::now();
    let mut chosen = false;
    let mut gfx_seen = gfx();
    let mut near = false;
    let mut pos = (0..plan.chain.len()).find(|&p| plan.step(p).is_some_and(&installed));
    // Mesures en jeu (0.14) et proposition d'un modèle supérieur, une par session au plus.
    let mut session = crate::mesures::Session::new(key.clone(), Instant::now());
    let mut merged_at = Instant::now();
    let mut proposal: Option<crate::conseil::Proposal> = None;
    let mut proposed = false;
    let mut mem = match &cfg.phrases {
        Some(p) => Memory::load(p, 50_000),
        None => Memory::new(50_000),
    };
    let (res_tx, res_rx) = mpsc::channel::<(u64, Result<Vec<String>, String>)>();
    let mut queue: VecDeque<Job> = VecDeque::new();
    let mut busy: HashMap<u64, Work> = HashMap::new();
    let mut running: Option<Running> = None;
    let mut next_wave = Instant::now();
    let mut lat: VecDeque<f64> = VecDeque::new();
    // Mémoire vidéo surveillée pendant le jeu (0.12.6).
    let mut vram_at = Instant::now();
    let say = |s: String| {
        let _ = ev.send(Event::Status(s));
    };
    let done = |id: u64, result: Result<String, String>, secs: f64, cached: bool| {
        let _ = ev.send(Event::Done { id, result, secs, cached });
    };
    say(format!("{} phrases déjà traduites en mémoire", mem.len()));
    if pos.is_none() {
        say(format!(
            "aucun modèle de la chaîne n'est installé ({}) : messages laissés tels quels",
            plan.downloads.iter().map(|m| m.name).collect::<Vec<_>>().join(", ")
        ));
    }
    loop {
        // Commandes : téléchargement fini, proposition acceptée ou écartée (bascules hors combat).
        for c in take_controls(in_combat()) {
            let mut replan = false;
            match c {
                Control::Refuse => {
                    if let Some(p) = proposal.take() {
                        crate::conseil::refuse(&key, &p.key());
                        say(format!("{} {} : proposition écartée, elle ne reviendra pas", p.model.name, p.place.label()));
                    }
                }
                Control::Accept => {
                    if let Some(p) = proposal.take() {
                        if catalog::installed_in(&cfg.models, p.model) {
                            crate::conseil::set_choice(&key, Some((p.model, p.place)));
                            say(format!("{} {} : essai demandé par le joueur", p.model.name, p.place.label()));
                            replan = true;
                        } else {
                            say(format!("{} : téléchargement avant l'essai ({})", p.model.name, catalog::go(p.model.size)));
                            crate::telechargements::want_for_trial(cfg.models.clone(), p.model, p.place);
                        }
                    }
                }
                Control::Prefer(id, place) => {
                    if let Some(m) = catalog::by_id(id) {
                        crate::conseil::set_choice(&key, Some((m, place)));
                        say(format!("{} {} : téléchargé, essai demandé par le joueur", m.name, place.label()));
                    }
                    replan = true;
                }
                Control::Replan => replan = true,
            }
            if replan {
                let free = bench::vram_sample().map(|(f, _, _)| available(&plan, running.as_ref(), f));
                plan = plan_with_choice(&cfg, Some(&key)).0;
                let best = gov.best_fit(&gov_steps(&plan, &installed), free, Instant::now());
                let current = running.as_ref().and_then(|r| r.label.clone());
                let wanted = best.and_then(|b| plan.step(b)).map(|c| c.label());
                if running.is_none() || current != wanted {
                    if running.take().is_some() {
                        for (_, w) in busy.drain() {
                            queue.push_front(w.job);
                        }
                        lat.clear();
                    }
                    pos = best;
                    if let Some(w) = wanted {
                        say(format!("meilleur modèle disponible : {w}"));
                    }
                } else if let (Some(r), Some(b)) = (running.as_mut(), best) {
                    // Même modèle : sa place dans la nouvelle chaîne.
                    r.pos = b;
                    pos = Some(b);
                }
            }
        }
        // Premier modèle, une fois le joueur en jeu (ou un message à traduire) :
        // - première fois dans ces conditions (version du jeu, définition ; aucune mesure dans
        //   `mesures.tsv`) : règle de la première installation, WoW compté 7 Go ;
        // - ensuite : la mémoire vidéo réellement libre en jeu (à l'écran de connexion, WoW en
        //   prend bien moins). Puis le gouverneur monte ou descend d'après les relevés.
        if !chosen {
            let ready = IN_WORLD.load(std::sync::atomic::Ordering::Relaxed)
                || !queue.is_empty()
                || started.elapsed() >= Duration::from_secs(120);
            if ready {
                chosen = true;
                let steps = gov_steps(&plan, &installed);
                let first = crate::mesures::load(&key).is_none();
                let room = plan.card.as_ref().map(|c| first_install_room(c.dedicated, c.others()));
                let measured = bench::vram_sample().map(|(free, _, _)| free);
                let (avail, why) = match (first, room, measured) {
                    (true, Some(r), _) => (Some(r), format!("première fois ici : WoW compté {} (règle), {} pour le modèle", crate::hw::gib(catalog::WOW_VRAM_REFERENCE), crate::hw::gib(r))),
                    (_, _, Some(f)) => (Some(f), format!("mémoire vidéo libre en jeu : {}", crate::hw::gib(f))),
                    _ => (None, "mémoire vidéo inconnue".to_string()),
                };
                let fit = gov.best_fit(&steps, avail, Instant::now());
                if fit.is_some() {
                    pos = fit;
                }
                say(format!(
                    "{why} (seuil de saturation {}) ; ensuite, les relevés en jeu font monter ou descendre le modèle",
                    crate::hw::gib(gov.saturation())
                ));
            }
        }
        // (Re)lancement sur l'étape courante.
        if running.is_none() && chosen {
            if let Some(p) = pos {
                say(format!("chargement de {}", step_label(&plan, p)));
                match launch(&cfg, &plan, p) {
                    Ok(r) => {
                        say(format!("prêt : {}", step_label(&plan, p)));
                        lat.clear();
                        running = Some(r);
                    }
                    Err((f, why)) => {
                        let c = plan.step(p).unwrap();
                        let mut again = String::new();
                        if f == Failure::Memory {
                            // Saturation au chargement : mis de côté un moment, pas interdit.
                            let d = gov.saturated(&c.label(), Instant::now());
                            pos = gov.lighter(&gov_steps(&plan, &installed), p, Instant::now());
                            again = format!(" (repris au plus tôt dans {} min si la place revient)", d.as_secs() / 60);
                        } else {
                            note(c, f, &why);
                            drop_choice(&key, c);
                            pos = plan.fallback_if(p, f, &installed);
                        }
                        say(match pos {
                            Some(q) => format!("{} : {why} ; repli sur {}{again}", c.label(), step_label(&plan, q)),
                            None => format!("{} : {why} ; plus aucune étape possible, messages laissés tels quels{again}", c.label()),
                        });
                        continue;
                    }
                }
            }
        }
        // Demandes nouvelles ; un brouillon remplace les brouillons plus anciens de sa saisie.
        let mut fresh: Vec<Job> = Vec::new();
        match jobs.recv_timeout(Duration::from_millis(30)) {
            Ok(j) => fresh.push(j),
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        fresh.extend(jobs.try_iter());
        for j in fresh {
            if j.ask.priority == Priority::Received {
                session.message(Instant::now());
            }
            if let (Priority::Draft, Some(g)) = (j.ask.priority, j.ask.group) {
                while let Some(k) = queue.iter().position(|q| q.ask.priority == Priority::Draft && q.ask.group == Some(g)) {
                    let old = queue.remove(k).unwrap();
                    done(old.id, Err("remplacé".into()), 0.0, false);
                }
            }
            queue.push_back(j);
        }
        // Résultats des traductions en cours.
        for (id, r) in res_rx.try_iter() {
            let Some(mut w) = busy.remove(&id) else { continue };
            let secs = w.started.elapsed().as_secs_f64();
            match r {
                Ok(outs) => {
                    let q = running.as_ref().map_or(1, |r| r.quality);
                    let mut it = outs.into_iter();
                    for (k, p) in w.pieces.iter().enumerate() {
                        if w.outs[k].is_none() {
                            let t = it.next().unwrap_or_default();
                            mem.put(p, w.job.ask.src, w.job.ask.tgt, &t, q);
                            w.outs[k] = Some(t);
                        }
                    }
                    done(id, Ok(w.text()), secs, false);
                    lat.push_back(secs);
                    if lat.len() > 10 {
                        lat.pop_front();
                    }
                }
                Err(e) => {
                    let dead = running.as_mut().is_none_or(|r| matches!(r.child.try_wait(), Ok(Some(_))));
                    if dead {
                        // Remis en tête : traduit par l'étape suivante.
                        queue.push_front(w.job);
                    } else {
                        done(id, Err(e), secs, false);
                    }
                }
            }
        }
        // Le serveur est-il toujours là ?
        if let Some(r) = running.as_mut() {
            if let Ok(Some(st)) = r.child.try_wait() {
                let p = r.pos;
                running = None;
                for (_, w) in busy.drain() {
                    queue.push_front(w.job);
                }
                let c = plan.step(p).unwrap();
                let why = format!("le serveur de traduction s'est arrêté ({st})");
                note(c, Failure::Crash, &why);
                drop_choice(&key, c);
                pos = plan.fallback_if(p, Failure::Crash, &installed);
                say(format!("{} : {why} ; repli", c.label()));
                continue;
            }
        }
        // Trop lent en jeu : étape suivante, plus rapide.
        let m = median(&lat);
        if lat.len() == 10 && m > catalog::MAX_LATENCY {
            if let Some(r) = running.take() {
                let c = plan.step(r.pos).unwrap();
                note(c, Failure::Slow, &format!("latence médiane {m:.2} s;latence={m:.2}"));
                drop_choice(&key, c);
                pos = plan.fallback_if(r.pos, Failure::Slow, &installed);
                say(format!("{} : trop lent en jeu ({m:.1} s par message) ; repli", c.label()));
                for (_, w) in busy.drain() {
                    queue.push_front(w.job);
                }
                lat.clear();
                continue;
            }
        }
        // Réglages graphiques de WoW changés (addon 0.14.2) : la place est réévaluée tout de suite.
        let g = gfx();
        if g != gfx_seen {
            if g != 0 {
                gov.graphics_changed();
                say("réglages graphiques de WoW modifiés : place du modèle réévaluée".into());
                vram_at = Instant::now().checked_sub(VRAM_CHECK).unwrap_or_else(Instant::now);
            }
            gfx_seen = g;
        }
        // Mémoire vidéo pendant le jeu (0.14.2, module `gouverneur`) : plus de réserve fixe. Près
        // de la saturation, une étape plus légère (même en combat : la mémoire revient au jeu) ;
        // hors combat, l'étape supérieure quand la place revient (une minute de suite).
        if vram_at.elapsed() >= if near { VRAM_CHECK_NEAR } else { VRAM_CHECK } {
            vram_at = Instant::now();
            let sample = bench::vram_sample();
            // Mesures en jeu : WoW ouvert, modèle chargé.
            match (sample, running.as_ref()) {
                (Some((free, wow, true)), Some(r)) => {
                    let fp = plan.step(r.pos).filter(|c| c.place == Place::Gpu).map_or(0, |c| c.footprint);
                    session.sample(Instant::now(), free, wow, fp, context() & CTX_INSTANCE != 0);
                }
                _ => session.pause(),
            }
            if merged_at.elapsed() >= Duration::from_secs(60) {
                merged_at = Instant::now();
                if let Some(o) = session.merge_into(crate::mesures::load(&key), catalog::now_secs()) {
                    let _ = crate::mesures::save(&o);
                    if !proposed && !in_combat() {
                        let active = running.as_ref().and_then(|r| plan.step(r.pos));
                        let refused = |w: &str| crate::conseil::refused(&key, w);
                        if let Some(p) = crate::conseil::propose(&plan, active, &o, &refused, &installed_model) {
                            proposed = true;
                            say(format!("proposition : {} {} ({})", p.model.name, p.place.label(), p.reason));
                            proposal = Some(p.clone());
                            let _ = ev.send(Event::Propose(p));
                        }
                    }
                }
            }
            if let (Some((free, _, true)), true) = (sample, chosen) {
                let steps = gov_steps(&plan, &installed);
                let cur = running.as_ref().map(|r| r.pos);
                let on_gpu = cur.and_then(|p| steps.get(p)).is_some_and(|s| s.gpu);
                near = on_gpu && free < 2 * crate::gouverneur::headroom(plan.card.as_ref().map_or(0, |c| c.dedicated));
                let mv = gov.sample(&steps, cur, free, in_combat(), Instant::now());
                let from = cur.and_then(|p| steps.get(p)).map(|s| s.label.clone()).unwrap_or_else(|| "aucun modèle".into());
                let to = |t: Option<usize>| t.and_then(|p| plan.step(p)).map_or("aucun modèle (le jeu passe d'abord)".to_string(), |_| step_label(&plan, t.unwrap()));
                let change = match mv {
                    crate::gouverneur::Move::Stay => None,
                    crate::gouverneur::Move::Down(t) => {
                        Some((t, format!("mémoire vidéo presque pleine ({} libres) : {from} -> {}", crate::hw::gib(free), to(t))))
                    }
                    crate::gouverneur::Move::Up(t) => Some((
                        Some(t),
                        format!("place revenue dans la mémoire vidéo ({} libres) : {from} -> {}", crate::hw::gib(free), to(Some(t))),
                    )),
                };
                if let Some((t, msg)) = change {
                    say(msg);
                    if running.take().is_some() {
                        for (_, w) in busy.drain() {
                            queue.push_front(w.job);
                        }
                        lat.clear();
                    }
                    pos = t;
                    continue;
                }
            }
        }
        let now = Instant::now();
        // Messages reçus qui ne seraient plus prêts à temps : affichés tels quels, tout de suite.
        let expected = Duration::from_secs_f64(if lat.is_empty() { 1.5 } else { median(&lat) });
        let gap = Duration::from_secs_f64(running.as_ref().map_or(0.0, |r| r.gap));
        let wave_busy = busy.values().any(|w| w.job.ask.priority == Priority::Received);
        for k in overdue(&queue, now, expected, next_wave, wave_busy, gap).into_iter().rev() {
            let j = queue.remove(k).unwrap();
            done(j.id, Err("trop ancien".into()), 0.0, false);
        }
        let Some(r) = running.as_ref() else {
            // Premier modèle pas encore choisi : la file attend (il est chargé au tour suivant).
            if !chosen {
                continue;
            }
            // Aucun modèle : rien ne sera traduit.
            for j in queue.drain(..) {
                done(j.id, Err("aucun modèle actif".into()), 0.0, false);
            }
            continue;
        };
        let wave_ok = now >= next_wave && !busy.values().any(|w| w.job.ask.priority == Priority::Received);
        let picked = pick(&queue, PARALLEL.saturating_sub(busy.len()), wave_ok);
        if picked.is_empty() {
            continue;
        }
        let mut chosen: Vec<Job> = Vec::new();
        let mut sorted = picked.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for k in sorted {
            chosen.push(queue.remove(k).unwrap());
        }
        chosen.sort_by_key(|j| (j.ask.priority, j.queued));
        if chosen.iter().any(|j| j.ask.priority == Priority::Received) {
            next_wave = now + Duration::from_secs_f64(r.gap);
        }
        for job in chosen {
            let w = Work::new(job, &mem, r.quality);
            let missing = w.missing();
            if missing.is_empty() {
                done(w.job.id, Ok(w.text()), 0.0, true);
                continue;
            }
            let (srv, tx, id) = (r.srv, res_tx.clone(), w.job.id);
            let (src, tgt) = (w.job.ask.src, w.job.ask.tgt);
            std::thread::spawn(move || {
                let r: Result<Vec<String>, String> = missing.iter().map(|p| translate_piece(srv, p, src, tgt)).collect();
                let _ = tx.send((id, r));
            });
            busy.insert(id, w);
        }
    }
}

/// Journal du serveur en usage réel.
pub fn log_path() -> PathBuf {
    crate::fonts::cache_root().join("traduction-serveur.log")
}

/// Mémoire des phrases traduites.
pub fn phrases_path() -> PathBuf {
    crate::fonts::cache_root().join("phrases.tsv")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, priority: Priority, age_ms: u64) -> Job {
        Job {
            id,
            ask: Ask { text: format!("m{id}"), src: "de", tgt: "fr", priority, group: None },
            queued: Instant::now() - Duration::from_millis(age_ms),
        }
    }

    #[test]
    fn first_install_counts_wow_at_7_gb_then_measures_decide() {
        use crate::gouverneur::{Governor, Step};
        const GIB: u64 = 1 << 30;
        let s = |label: &str, gpu: bool, fp: u64| Step { label: label.into(), gpu, footprint: fp, usable: true };
        let steps = [
            s("7B carte", true, 46 * GIB / 10),
            s("4B carte", true, 285 * GIB / 100),
            s("1.7B carte", true, 115 * GIB / 100),
            s("1.7B processeur", false, 0),
        ];
        let g = Governor::new(16 * GIB);
        let now = Instant::now();
        // Carte de 16 Go, 1,4 Go pris par les autres programmes, WoW compté 7 Go : 7,6 Go pour le
        // modèle -> le 7B (4,6 Go et 0,9 Go de marge pour remonter) à la première installation.
        let room = first_install_room(16 * GIB, 14 * GIB / 10);
        assert_eq!(room / (GIB / 10), 76);
        assert_eq!(g.best_fit(&steps, Some(room), now), Some(0));
        // Carte de 8 Go : 0,6 Go -> sur le processeur d'abord ; les relevés le feront monter.
        assert_eq!(g.best_fit(&steps, Some(first_install_room(8 * GIB, 4 * GIB / 10)), now), Some(3));
        // Ensuite, la mémoire réellement libre décide (WoW au maximum : 1,2 Go libres).
        assert_eq!(g.best_fit(&steps, Some(12 * GIB / 10), now), Some(3));
    }

    #[test]
    fn player_requests_first_and_received_messages_by_waves() {
        let q: VecDeque<Job> = [
            job(1, Priority::Received, 900),
            job(2, Priority::Received, 800),
            job(3, Priority::Draft, 500),
            job(4, Priority::Send, 100),
            job(5, Priority::Received, 50),
        ]
        .into_iter()
        .collect();
        // Vague permise, 4 emplacements libres : envoi, brouillon, puis les deux plus anciens reçus.
        assert_eq!(pick(&q, 4, true), vec![3, 2, 0, 1]);
        // Hors vague : seulement les demandes du joueur.
        assert_eq!(pick(&q, 4, false), vec![3, 2]);
        assert_eq!(pick(&q, 1, true), vec![3]);
        assert!(pick(&q, 0, true).is_empty());
        // Un message reçu qui ne serait pas prêt avant 60 s n'est pas lancé.
        let expected = Duration::from_millis(1500);
        assert!(!too_late(&q[0], Instant::now(), expected));
        assert!(!too_late(&job(6, Priority::Received, 3000), Instant::now(), expected));
        assert!(too_late(&job(6, Priority::Received, 59_000), Instant::now(), expected));
        assert!(!too_late(&job(7, Priority::Send, 9000), Instant::now(), expected));
    }

    #[test]
    fn a_flood_of_received_messages_is_given_up_early() {
        let now = Instant::now();
        // 170 messages reçus d'un coup : 1,5 s par vague de 4. Les 40 premières vagues (160
        // messages) finissent avant 60 s, les suivants non : abandonnés tout de suite.
        let q: VecDeque<Job> = (0..170).map(|i| job(i, Priority::Received, 0)).chain([job(999, Priority::Send, 0)]).collect();
        let late = overdue(&q, now, Duration::from_millis(1500), now, false, Duration::ZERO);
        assert_eq!(late, (160..170).collect::<Vec<_>>());
        // Une vague en cours et des vagues espacées de 15 s : quatre vagues passent.
        let late = overdue(&q, now, Duration::from_millis(1500), now, true, Duration::from_secs(15));
        assert_eq!(late, (16..170).collect::<Vec<_>>());
        // Peu de messages : rien n'est abandonné.
        let few: VecDeque<Job> = (0..3).map(|i| job(i, Priority::Received, 500)).collect();
        assert!(overdue(&few, now, Duration::from_millis(1500), now, false, Duration::ZERO).is_empty());
    }

    #[test]
    fn phrases_are_remembered_on_disk_and_reused() {
        let dir = std::env::temp_dir().join(format!("ct-mem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("phrases.tsv");
        let _ = std::fs::remove_file(&path);
        let mut m = Memory::load(&path, 100);
        m.put("WTS [1]  pst", "en", "fr", "VDS [1] mp", 3);
        m.put("[2] [1] cheap, [3]", "en", "fr", "[2] [1] pas cher, [3]", 3);
        m.put("ty\tall", "en", "fr", "merci\nà tous", 1);
        // Relu à la partie suivante.
        let m = Memory::load(&path, 100);
        assert_eq!(m.get("WTS [1] pst", "en", "fr", 3), Some("VDS [1] mp"));
        assert_eq!(m.get("ty\tall", "en", "fr", 1), Some("merci\nà tous"));
        // Un modèle meilleur ne reprend pas la traduction d'un moins bon.
        assert_eq!(m.get("ty\tall", "en", "fr", 2), None);
        assert_eq!(m.get("WTS [1] pst", "en", "de", 1), None);
        // Le lien change, la phrase est la même : elle ressert.
        let w = Work::new(
            Job {
                id: 1,
                ask: Ask {
                    text: "WTS |cffa335ee|Hitem:2::::|h[Épée]|h|r cheap, pst".into(),
                    src: "en",
                    tgt: "fr",
                    priority: Priority::Received,
                    group: None,
                },
                queued: Instant::now(),
            },
            &m,
            3,
        );
        assert!(w.missing().is_empty());
        // Acronymes traduits par le dictionnaire (0.12.4), le reste par la mémoire.
        assert_eq!(w.text(), "Vends |cffa335ee|Hitem:2::::|h[Épée]|h|r pas cher, MP");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn messages_split_into_reusable_sentences() {
        assert_eq!(sentences("merci pour le groupe. bonne soirée !"), vec!["merci pour le groupe.", "bonne soirée !"]);
        assert_eq!(sentences("LF [1] pour [2], on part dans 10 min"), vec!["LF [1] pour [2], on part dans 10 min"]);
        assert_eq!(sentences("gg. :)"), vec!["gg. :)"]);
        assert_eq!(sentences("3.5 po chacun"), vec!["3.5 po chacun"]);
    }
}
