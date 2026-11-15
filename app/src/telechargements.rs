//! Téléchargements de fond de l'application (0.14) : moteur llama.cpp, puis les modèles qui
//! peuvent tourner sur ce PC (le pack low d'abord, la traduction démarre dès qu'il est là), un à
//! la fois. WoW ouvert, le débit est bridé (un quart du débit mesuré WoW fermé, entre 1 et 8 Mo/s)
//! pour ne pas gêner le jeu en ligne ; WoW ouvert ou fermé pendant un téléchargement, il reprend
//! au nouveau débit là où il en était. À la fin de chaque modèle, le moteur recalcule sa chaîne
//! (hors combat) : le meilleur modèle installé devient actif.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::catalog::{self, Model, Place};
use crate::engine::{self, Control};

const MB: u64 = 1 << 20;

/// Après un téléchargement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum After {
    /// Chaîne recalculée : le meilleur modèle installé devient actif.
    Replan,
    /// Proposition acceptée : ce modèle est essayé.
    Prefer(Place),
}

#[derive(Clone, Debug)]
enum What {
    Runtime(String),
    Model(&'static Model),
}

#[derive(Clone, Debug)]
struct Job {
    what: What,
    dir: PathBuf,
    after: After,
}

/// État des téléchargements, pour l'icône, le panneau sur le jeu et le journal.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Téléchargement en cours ou en attente.
    pub active: bool,
    /// Ce qui se télécharge maintenant.
    pub current: Option<String>,
    /// Son identifiant (« moteur » ou celui du modèle), sa progression à lui (0.16.6) et son rang
    /// dans la série (1 sur 3...).
    pub current_id: Option<String>,
    pub item_got: u64,
    pub item_total: u64,
    pub rank: usize,
    /// En attente, dans l'ordre (identifiants).
    pub waiting: Vec<String>,
    /// Octets reçus et attendus pour l'ensemble de la série en cours.
    pub got: u64,
    pub total: u64,
    /// Débit actuel (octets par seconde) et bridage (0 : aucun).
    pub speed: f64,
    pub limit: u64,
    /// Dernier problème (téléchargement repris ensuite, ou abandonné).
    pub problem: Option<String>,
    /// Terminés depuis le lancement de l'application.
    pub done: Vec<String>,
}

impl Status {
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            0
        } else {
            (self.got as f64 * 100.0 / self.total as f64).round().min(100.0) as u32
        }
    }

    /// « 2 sur 3 : TranslateGemma 4B, 37 % (0,86 Go sur 2,32 Go) » : le fichier en cours, avec sa
    /// taille à lui (0.16.6 ; avant, le total de la série à côté de son nom prêtait à confusion).
    pub fn line(&self) -> String {
        let name = self.current.as_deref().unwrap_or("modèles");
        let count = self.rank + self.waiting.len();
        let head = if count > 1 { format!("{} sur {count} : {name}", self.rank) } else { name.to_string() };
        let (got, total) = if self.item_total > 0 { (self.item_got, self.item_total) } else { (self.got, self.total) };
        let pct = if total == 0 { 0 } else { (got as f64 * 100.0 / total as f64).round().min(100.0) as u32 };
        format!(
            "{head}, {pct} % ({} sur {}){}",
            catalog::go(got),
            catalog::go(total),
            if self.limit > 0 { ", bridé pendant le jeu" } else { "" }
        )
    }

    /// État pour la fenêtre des modèles (autre processus) : `telechargements.json`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "pid": std::process::id(),
            "maj": catalog::now_secs(),
            "actif": self.active,
            "courant": self.current_id,
            "recu": self.item_got,
            "taille": self.item_total,
            "debit": self.speed,
            "bride": self.limit > 0,
            "attente": self.waiting,
            "serie_recu": self.got,
            "serie_total": self.total,
            "probleme": self.problem,
        })
    }
}

/// Fichier d'état des téléchargements de fond, lu par la fenêtre des modèles (0.16.6).
pub fn status_file() -> PathBuf {
    crate::fonts::cache_root().join("telechargements.json")
}

/// État écrit d'un coup (fichier temporaire puis renommé) : jamais lu à moitié.
fn publish(st: &Status) {
    let path = status_file();
    let tmp = path.with_extension("json.tmp");
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::write(&tmp, st.to_json().to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Téléchargements de fond vus par la fenêtre des modèles : l'état publié par l'application, s'il
/// est récent (moins de 15 s) et encore actif.
pub fn read_published() -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(status_file()).ok()?).ok()?;
    let fresh = catalog::now_secs().saturating_sub(v["maj"].as_u64()?) <= 15;
    (fresh && v["actif"].as_bool() == Some(true)).then_some(v)
}

fn id_of(w: &What) -> String {
    match w {
        What::Runtime(_) => "moteur".to_string(),
        What::Model(m) => m.id.to_string(),
    }
}

struct State {
    queue: VecDeque<Job>,
    status: Status,
    /// Octets déjà terminés dans la série en cours.
    finished: u64,
    running: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    queue: VecDeque::new(),
    status: Status {
        active: false,
        current: None,
        current_id: None,
        item_got: 0,
        item_total: 0,
        rank: 0,
        waiting: Vec::new(),
        got: 0,
        total: 0,
        speed: 0.0,
        limit: 0,
        problem: None,
        done: Vec::new(),
    },
    finished: 0,
    running: false,
});

/// Test « WoW est ouvert », fourni par l'application (Windows) ; faux par défaut.
static WOW_PROBE: OnceLock<fn() -> bool> = OnceLock::new();
/// Débit mesuré WoW fermé (octets par seconde), pour le bridage.
static FULL_SPEED: AtomicU64 = AtomicU64::new(0);

pub fn set_wow_probe(f: fn() -> bool) {
    let _ = WOW_PROBE.set(f);
}

fn wow_open() -> bool {
    WOW_PROBE.get().is_some_and(|f| f())
}

/// Débit permis : aucun bridage WoW fermé ; WoW ouvert, un quart du débit mesuré WoW fermé, entre
/// 1 et 8 Mo/s (3 Mo/s s'il n'a pas encore été mesuré).
pub fn limit_for(wow: bool, full_speed: u64) -> u64 {
    if !wow {
        0
    } else if full_speed == 0 {
        3 * MB
    } else {
        (full_speed / 4).clamp(MB, 8 * MB)
    }
}

pub fn status() -> Status {
    STATE.lock().map(|s| s.status.clone()).unwrap_or_default()
}

/// Moteur et modèles manquants mis en file (le moteur d'abord, les modèles dans l'ordre donné),
/// puis la chaîne recalculée à la fin de chacun. Renvoie le nombre d'octets à télécharger.
pub fn want(dir: PathBuf, runtime: Option<&str>, models: &[&'static Model]) -> u64 {
    let mut jobs = Vec::new();
    let mut bytes = 0;
    if let Some(r) = runtime.filter(|r| !catalog::runtime_present(&catalog::runtime_dir(), r)) {
        jobs.push(Job { what: What::Runtime(r.to_string()), dir: dir.clone(), after: After::Replan });
        bytes += 60 * MB;
    }
    for m in models.iter().filter(|m| !catalog::installed_in(&dir, m)) {
        jobs.push(Job { what: What::Model(m), dir: dir.clone(), after: After::Replan });
        bytes += m.size;
    }
    enqueue(jobs, bytes);
    bytes
}

/// Un modèle pour une proposition acceptée : essayé une fois téléchargé.
pub fn want_for_trial(dir: PathBuf, m: &'static Model, place: Place) {
    if catalog::installed_in(&dir, m) {
        engine::post(Control::Prefer(m.id, place));
        return;
    }
    enqueue(vec![Job { what: What::Model(m), dir, after: After::Prefer(place) }], m.size);
}

fn enqueue(jobs: Vec<Job>, bytes: u64) {
    if jobs.is_empty() {
        return;
    }
    let start = {
        let Ok(mut s) = STATE.lock() else { return };
        if !s.status.active {
            s.finished = 0;
            s.status.got = 0;
            s.status.total = 0;
            s.status.rank = 0;
        }
        for j in jobs {
            let dup = s.queue.iter().any(|q| same(&q.what, &j.what));
            if !dup {
                s.queue.push_back(j);
            }
        }
        s.status.total += bytes;
        s.status.active = true;
        s.status.waiting = s.queue.iter().map(|j| id_of(&j.what)).collect();
        publish(&s.status);
        let start = !s.running;
        s.running = true;
        start
    };
    if start {
        std::thread::spawn(worker);
    }
}

fn same(a: &What, b: &What) -> bool {
    match (a, b) {
        (What::Runtime(x), What::Runtime(y)) => x == y,
        (What::Model(x), What::Model(y)) => x.id == y.id,
        _ => false,
    }
}

fn label(w: &What) -> String {
    match w {
        What::Runtime(r) => format!("moteur llama.cpp ({r})"),
        What::Model(m) => m.name.to_string(),
    }
}

fn worker() {
    loop {
        let job = {
            let Ok(mut s) = STATE.lock() else { return };
            match s.queue.pop_front() {
                Some(j) => {
                    s.status.current = Some(label(&j.what));
                    s.status.current_id = Some(id_of(&j.what));
                    s.status.rank += 1;
                    s.status.item_got = 0;
                    s.status.item_total = match &j.what {
                        What::Runtime(_) => 60 * MB,
                        What::Model(m) => crate::modeles_maj::known_size(m.file).unwrap_or(m.size),
                    };
                    s.status.waiting = s.queue.iter().map(|q| id_of(&q.what)).collect();
                    publish(&s.status);
                    j
                }
                None => {
                    s.running = false;
                    s.status.active = false;
                    s.status.current = None;
                    s.status.current_id = None;
                    s.status.waiting.clear();
                    publish(&s.status);
                    catalog::RATE_LIMIT.store(0, Ordering::Relaxed);
                    return;
                }
            }
        };
        crate::say!("[téléchargement] {} : début", label(&job.what));
        let r = run(&job);
        let Ok(mut s) = STATE.lock() else { return };
        match r {
            Ok(size) => {
                s.finished += size;
                s.status.got = s.finished;
                s.status.item_got = s.status.item_total;
                s.status.done.push(label(&job.what));
                s.status.problem = None;
                publish(&s.status);
                drop(s);
                crate::say!("[téléchargement] {} : terminé", label(&job.what));
                match (job.after, &job.what) {
                    (After::Prefer(p), What::Model(m)) => engine::post(Control::Prefer(m.id, p)),
                    _ => engine::post(Control::Replan),
                }
            }
            Err(e) => {
                crate::say!("[téléchargement] {} : abandonné ({e})", label(&job.what));
                s.status.problem = Some(format!("{} : {e}", label(&job.what)));
                publish(&s.status);
            }
        }
    }
}

/// Un téléchargement, repris au nouveau débit quand WoW s'ouvre ou se ferme. Renvoie sa taille.
fn run(job: &Job) -> Result<u64, String> {
    let cancel = AtomicBool::new(false);
    loop {
        let wow = wow_open();
        let limit = limit_for(wow, FULL_SPEED.load(Ordering::Relaxed));
        catalog::RATE_LIMIT.store(limit, Ordering::Relaxed);
        if let Ok(mut s) = STATE.lock() {
            s.status.limit = limit;
        }
        cancel.store(false, Ordering::Relaxed);
        let mut checked = Instant::now();
        let mut published = Instant::now();
        let mut report = |p: &catalog::Progress| {
            if let Ok(mut s) = STATE.lock() {
                s.status.got = s.finished + p.got;
                s.status.speed = p.speed;
                s.status.item_got = p.got;
                if p.total > 0 {
                    s.status.item_total = p.total;
                }
                // La fenêtre des modèles suit la progression réelle (0.16.6).
                if published.elapsed().as_millis() >= 1000 {
                    published = Instant::now();
                    publish(&s.status);
                }
            }
            if !wow && p.speed > 0.0 {
                FULL_SPEED.store(p.speed as u64, Ordering::Relaxed);
            }
            // WoW ouvert ou fermé depuis : reprise au nouveau débit.
            if checked.elapsed().as_secs() >= 2 {
                checked = Instant::now();
                if wow_open() != wow {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        };
        let r = match &job.what {
            What::Runtime(rt) => catalog::download_runtime_with(rt, &catalog::runtime_dir(), &cancel, &mut report).map(|_| 60 * MB),
            What::Model(m) => catalog::download_model_with(m, &job.dir, &cancel, &mut report).map(|_| m.size),
        };
        match r {
            Err(e) if e == catalog::CANCELLED && cancel.load(Ordering::Relaxed) => continue,
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_while_playing() {
        assert_eq!(limit_for(false, 50 * MB), 0);
        assert_eq!(limit_for(true, 0), 3 * MB);
        assert_eq!(limit_for(true, 20 * MB), 5 * MB);
        assert_eq!(limit_for(true, 100 * MB), 8 * MB);
        assert_eq!(limit_for(true, 2 * MB), MB);
        let s = Status { current: Some("TranslateGemma 4B".into()), got: 2 << 30, total: 6 << 30, limit: MB, ..Default::default() };
        assert_eq!(s.percent(), 33);
        assert!(s.line().starts_with("TranslateGemma 4B, 33 % ("), "{}", s.line());
        assert!(s.line().ends_with("bridé pendant le jeu"));
        // 0.16.6 : le fichier en cours avec sa taille à lui et son rang, pas le total de la série
        // (l'icône annonçait « SalamandraTA 7B v3 … sur 7,87 Go », la fenêtre 4,52 Go).
        let s = Status {
            current: Some("SalamandraTA 7B v3".into()),
            current_id: Some("salamandrata-7b".into()),
            rank: 2,
            waiting: vec!["translategemma-4b".into()],
            item_got: 4_850_334_784 / 4,
            item_total: 4_850_334_784,
            got: 1_000_000_000 + 4_850_334_784 / 4,
            total: 8_445_000_000,
            ..Default::default()
        };
        assert_eq!(s.line(), "2 sur 3 : SalamandraTA 7B v3, 25 % (1,13 Go sur 4,52 Go)");
        let j = s.to_json();
        assert_eq!((j["courant"].as_str(), j["taille"].as_u64(), j["attente"][0].as_str()), (Some("salamandrata-7b"), Some(4_850_334_784), Some("translategemma-4b")));
    }
}
