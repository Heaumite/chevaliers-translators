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

    /// « TranslateGemma 4B : 37 % (2,1 Go sur 5,8 Go) ».
    pub fn line(&self) -> String {
        format!(
            "{} : {} % ({} sur {}){}",
            self.current.as_deref().unwrap_or("modèles"),
            self.percent(),
            catalog::go(self.got),
            catalog::go(self.total),
            if self.limit > 0 { ", débit bridé pendant le jeu" } else { "" }
        )
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
        }
        for j in jobs {
            let dup = s.queue.iter().any(|q| same(&q.what, &j.what));
            if !dup {
                s.queue.push_back(j);
            }
        }
        s.status.total += bytes;
        s.status.active = true;
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
                    j
                }
                None => {
                    s.running = false;
                    s.status.active = false;
                    s.status.current = None;
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
                s.status.done.push(label(&job.what));
                s.status.problem = None;
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
        let mut report = |p: &catalog::Progress| {
            if let Ok(mut s) = STATE.lock() {
                s.status.got = s.finished + p.got;
                s.status.speed = p.speed;
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
        assert!(s.line().starts_with("TranslateGemma 4B : 33 % ("), "{}", s.line());
        assert!(s.line().ends_with("débit bridé pendant le jeu"));
    }
}
