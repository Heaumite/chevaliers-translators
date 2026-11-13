//! Interface de gestion des modèles : une page servie en local, affichée dans la fenêtre de
//! l'application (WebView2, livré avec Windows 10 et 11) ; à défaut dans Edge en mode
//! application, sinon dans le navigateur.
//!
//! La page montre la mémoire vidéo de la carte pendant le jeu, les modèles du plus léger au
//! plus lourd (pack low par défaut et repli, HIGH = le meilleur pour ce PC), télécharge à la
//! demande, active un seul modèle à la fois et supprime ceux qu'on ne veut plus.
//! Les appels de la page portent un jeton tiré au lancement (seule la fenêtre ouverte par
//! l'application peut agir).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::catalog::{self, Model, Place, Trial, Trials, Verdict, LOW_PACK};
use crate::hw::{Hw, GIB};

const PAGE: &str = include_str!("../ui/index.html");
/// Logo et bannière de Chevaliers Translators (tirés de media/).
const LOGO: &[u8] = include_bytes!("../ui/logo.png");
const BANNIERE: &[u8] = include_bytes!("../ui/banniere.jpg");
/// Identifiant du moteur llama.cpp dans les téléchargements.
const ENGINE: &str = "moteur";

pub struct Options {
    pub dir: PathBuf,
    pub window: bool,
    pub demo: bool,
    pub port: u16,
}

#[derive(Clone, Debug)]
struct Dl {
    got: u64,
    total: u64,
    speed: f64,
    /// « en_cours », « fini », « erreur », « annule ».
    state: &'static str,
    message: String,
    cancel: Arc<AtomicBool>,
}

struct Shared {
    demo: bool,
    dir: PathBuf,
    hw: Hw,
    dls: HashMap<String, Dl>,
    last_hit: Instant,
    hits: u64,
    // Démonstration : état simulé, sans fichier ni réseau.
    demo_installed: HashSet<&'static str>,
    demo_active: Option<String>,
    demo_runtime: bool,
}

type State = Arc<Mutex<Shared>>;

/// PC de référence (Radeon RX 6950 XT 16 Go, WoW en jeu) pour la démonstration.
fn demo_hw() -> Hw {
    use crate::hw::{Cpu, Gpu, Runtimes};
    Hw {
        cpu: Cpu {
            name: "AMD Ryzen 7 5800X3D 8-Core Processor".into(),
            logical: 16,
            physical: Some(8),
            avx2: true,
            ..Default::default()
        },
        ram_total: 64 * GIB,
        ram_avail: 48 * GIB,
        gpus: vec![Gpu {
            name: "AMD Radeon RX 6950 XT".into(),
            vendor_id: 0x1002,
            dedicated: 16 * GIB - 16 * 1024 * 1024,
            used: Some(69 * GIB / 10),
            used_by_wow: Some(56 * GIB / 10),
            ..Default::default()
        }],
        runtimes: Runtimes { vulkan: true, ..Default::default() },
        wow: vec!["WowClassic.exe".into()],
    }
}

fn demo_trials() -> Trials {
    let mut t = Trials::new();
    t.insert(
        (catalog::ON_DEMAND[1].file.to_lowercase(), Place::Gpu),
        Trial {
            verdict: Verdict::Crash,
            when: catalog::now_secs() - 3600,
            drop: Some(0.186),
            detail: "pilote graphique réinitialisé 3 fois".into(),
        },
    );
    t
}

impl Shared {
    fn installed(&self, m: &Model) -> bool {
        if self.demo {
            self.demo_installed.contains(m.id)
        } else {
            catalog::installed_in(&self.dir, m)
        }
    }
    fn offer(&self) -> catalog::Offer {
        if self.demo {
            let measured = |f: &str| (f == catalog::ON_DEMAND[1].file).then_some(43 * GIB / 10);
            catalog::offer_with(&self.hw, &|m| self.installed(m), &measured, &demo_trials())
        } else {
            catalog::offer_with(&self.hw, &|m| self.installed(m), &catalog::measured_footprint, &catalog::read_trials())
        }
    }
    /// Estimation sans test : modèle conseillé, chaîne de repli, téléchargements.
    fn plan(&self) -> crate::plan::Plan {
        if self.demo {
            let measured = |f: &str| (f == catalog::ON_DEMAND[1].file).then_some(43 * GIB / 10);
            let cond = crate::plan::Conditions {
                size: Some((3840, 1600)),
                source: "démonstration".into(),
                ..crate::plan::Conditions::worst()
            };
            crate::plan::estimate(&self.hw, &measured, &demo_trials(), cond)
        } else {
            crate::plan::for_this_pc(&self.hw, None)
        }
    }
    fn active(&self) -> Option<String> {
        if self.demo {
            self.demo_active.clone()
        } else {
            catalog::read_active()
        }
    }
    fn runtime_ready(&self, runtime: &str) -> bool {
        if self.demo {
            self.demo_runtime
        } else {
            catalog::runtime_present(&catalog::runtime_dir(), runtime)
        }
    }
    fn running(&self) -> bool {
        self.dls.values().any(|d| d.state == "en_cours")
    }
}

fn dl_json(d: Option<&Dl>) -> Value {
    match d {
        None => Value::Null,
        Some(d) => json!({
            "recu": d.got, "total": d.total, "debit": d.speed, "etat": d.state, "message": d.message,
        }),
    }
}

fn trial_json(t: &Trial, lang: &str) -> Value {
    json!({
        "verdict": t.verdict.code(),
        "resume": tr(lang, &t.summary()),
        "date": t.when,
        "baisse": t.drop,
    })
}

/// Explication de l'application dans la langue de la page (0.16.1).
fn tr(lang: &str, text: &str) -> String {
    crate::textes::translate(lang, text)
}

fn state_json(s: &Shared, lang: &str) -> Value {
    let o = s.offer();
    let p = s.plan();
    let active = s.active();
    let (eff, why) = catalog::effective_active(active.as_deref(), &o);
    let models: Vec<Value> = o
        .entries
        .iter()
        .map(|e| {
            let m = e.model;
            json!({
                "id": m.id,
                "nom": m.name,
                "fichier": m.file,
                "taille": m.size,
                "langues": tr(lang, m.langs.label()),
                "nb_langues": if m.langs == catalog::Langs::Low { 6 } else { 23 },
                // Langues du pack low, nommées par la page dans sa langue.
                "codes_langues": if m.langs == catalog::Langs::Low { json!(["fr", "en", "es", "it", "de", "ru"]) } else { json!([]) },
                "licence": m.license,
                "qualite": m.quality,
                "empreinte": e.footprint,
                "mesuree": e.measured,
                "lieu": e.place.map(|p| p.code()),
                "raison": tr(lang, &e.reason),
                "essai": e.trial.as_ref().map(|t| trial_json(t, lang)),
                "echec_carte": e.gpu_failed.as_ref().map(|t| tr(lang, &t.summary())),
                "installe": e.installed,
                "low": m.id == LOW_PACK.id,
                "high": p.high().map(|h| h.id) == Some(m.id),
                "conseille": p.downloads.iter().any(|d| d.id == m.id),
                "rang": p.rank(m.id),
                "estimation": p.best_for(m.id).map(|c| json!({
                    "lieu": c.place.code(),
                    "niveau": c.level.label(),
                    "latence": c.latency.is_finite().then_some(c.latency),
                    "capacite": c.capacity.is_finite().then_some(c.capacity),
                    "niveau_code": format!("{:?}", c.level).to_lowercase(),
                    "raison": tr(lang, &c.why),
                    "source": tr(lang, &c.source),
                })),
                "actif": eff.model.id == m.id,
                "dl": dl_json(s.dls.get(m.id)),
            })
        })
        .collect();
    let card = o.card.as_ref().map(|c| {
        json!({
            "nom": c.name, "total": c.dedicated, "autres": c.others(), "wow": c.wow,
            "wow_mesure": c.wow_measured, "wow_lance": c.wow_running, "libre_jeu": c.free_in_game,
            "marge": crate::gouverneur::saturation(c.dedicated),
        })
    });
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "demo": s.demo,
        "carte": card,
        "processeur": { "nom": s.hw.cpu.name, "fils": o.cpu_threads, "ram_libre": o.ram_free },
        "modeles": models,
        // « Modèle : raison » : la raison traduite.
        "repli": why.map(|w| match w.split_once(" : ") {
            Some((name, r)) => format!("{name} : {}", tr(lang, r)),
            None => tr(lang, &w),
        }),
        "moteur": {
            "nom": o.runtime,
            "pret": s.runtime_ready(o.runtime),
            "taille": catalog::runtime_size(o.runtime),
            "dl": dl_json(s.dls.get(ENGINE)),
        },
        "dossier": s.dir.display().to_string(),
        "disque_libre": catalog::free_disk(&s.dir),
        "notes": o.notes.iter().map(|n| tr(lang, n)).collect::<Vec<_>>(),
        "langue": crate::textes::normalize(lang),
        // Mise à jour publiée sur GitHub (0.16.1) : bouton et note de mise à jour dans la page ;
        // nouveautés de la version qui vient d'être installée.
        "maj": crate::maj::read_state(env!("CARGO_PKG_VERSION")),
        "nouveautes": crate::maj::read_installed(env!("CARGO_PKG_VERSION")),
        "estimation": {
            "conditions": tr(lang, &p.conditions.label()),
            "chaine": p.chain.iter().map(|&i| tr(lang, &p.candidates[i].label())).collect::<Vec<_>>(),
            "a_telecharger": p.downloads.iter().filter(|m| !s.installed(m)).map(|m| m.size).sum::<u64>(),
        },
    })
}

// --- Téléchargements -----------------------------------------------------------------------

fn start_download(state: &State, id: &str) -> Result<(), String> {
    let mut s = state.lock().unwrap();
    if s.dls.get(id).is_some_and(|d| d.state == "en_cours") {
        return Ok(());
    }
    let (model, runtime) = if id == ENGINE {
        (None, s.offer().runtime)
    } else {
        let o = s.offer();
        let e = o.entry(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
        if !e.eligible() {
            return Err(format!("{} n'est pas proposé sur ce PC : {}", e.model.name, e.reason));
        }
        (Some(e.model), o.runtime)
    };
    let total = model.map_or(catalog::runtime_size(runtime), |m| m.size);
    let cancel = Arc::new(AtomicBool::new(false));
    s.dls.insert(
        id.to_string(),
        Dl { got: 0, total, speed: 0.0, state: "en_cours", message: String::new(), cancel: cancel.clone() },
    );
    let (demo, dir) = (s.demo, s.dir.clone());
    drop(s);
    let st = state.clone();
    let key = id.to_string();
    let name = model.map_or(format!("moteur llama.cpp ({runtime})"), |m| m.name.to_string());
    crate::say!("Téléchargement : {name}");
    std::thread::spawn(move || {
        let mut report = |p: &catalog::Progress| {
            if let Some(d) = st.lock().unwrap().dls.get_mut(&key) {
                d.got = p.got;
                if p.total > 0 {
                    d.total = p.total;
                }
                d.speed = p.speed;
                d.message = p.problem.clone();
            }
        };
        let r = if demo {
            simulate(total, &cancel, &mut report)
        } else {
            match model {
                Some(m) => catalog::download_model_with(m, &dir, &cancel, &mut report),
                None => catalog::download_runtime_with(runtime, &catalog::runtime_dir(), &cancel, &mut report).map(|_| ()),
            }
        };
        let mut s = st.lock().unwrap();
        if demo && r.is_ok() {
            match model {
                Some(m) => {
                    s.demo_installed.insert(m.id);
                }
                None => s.demo_runtime = true,
            }
        }
        if let Some(d) = s.dls.get_mut(&key) {
            match r {
                Ok(()) => {
                    d.state = "fini";
                    d.got = d.total;
                    d.message.clear();
                    crate::say!("  {name} : prêt");
                }
                Err(e) if e == catalog::CANCELLED => {
                    d.state = "annule";
                    d.message = e;
                    crate::say!("  {name} : annulé");
                }
                Err(e) => {
                    crate::say!("  {name} : {e}");
                    d.state = "erreur";
                    d.message = e;
                }
            }
        }
    });
    Ok(())
}

/// Téléchargement simulé (démonstration) : 90 Mo/s.
fn simulate(total: u64, cancel: &AtomicBool, report: &mut dyn FnMut(&catalog::Progress)) -> Result<(), String> {
    let mut p = catalog::Progress { total, attempt: 1, ..Default::default() };
    while p.got < total {
        if cancel.load(Ordering::Relaxed) {
            return Err(catalog::CANCELLED.into());
        }
        std::thread::sleep(Duration::from_millis(100));
        // 90 Mo/s ; bien plus vite pendant les tests.
        let step: u64 = if cfg!(test) { 512 * 1_048_576 } else { 9 * 1_048_576 };
        p.speed = step as f64 * 10.0;
        p.got = (p.got + step).min(total);
        report(&p);
    }
    Ok(())
}

/// Langue choisie dans la page (gardée à côté du dossier des modèles : l'adresse de la page change
/// à chaque ouverture, la mémoire du navigateur ne la retrouverait pas).
fn lang_file(dir: &std::path::Path) -> std::path::PathBuf {
    dir.parent().unwrap_or(dir).join("langue-fenetre.txt")
}

fn saved_lang(dir: &std::path::Path) -> Option<String> {
    let l = std::fs::read_to_string(lang_file(dir)).ok()?;
    let l = l.trim();
    crate::textes::LANGS.contains(&l).then(|| l.to_string())
}

fn act(state: &State, action: &str, id: &str) -> Result<(), String> {
    match action {
        "langue" => {
            if !crate::textes::LANGS.contains(&id) {
                return Err(format!("langue inconnue : {id}"));
            }
            let dir = state.lock().unwrap().dir.clone();
            std::fs::write(lang_file(&dir), id).map_err(|e| e.to_string())
        }
        // Bouton « Installer la mise à jour » : l'application (icône) l'installe et redémarre.
        "maj" => crate::maj::request_install(),
        "maj_vu" => {
            crate::maj::dismiss_installed();
            Ok(())
        }
        "telecharger" => start_download(state, id),
        "annuler" => {
            if let Some(d) = state.lock().unwrap().dls.get(id) {
                d.cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        }
        "activer" => {
            let mut s = state.lock().unwrap();
            let o = s.offer();
            let e = o.entry(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
            if !e.installed {
                return Err(format!("{} n'est pas encore téléchargé", e.model.name));
            }
            if !e.eligible() {
                return Err(format!("{} : {}", e.model.name, e.reason));
            }
            if s.demo {
                s.demo_active = Some(id.to_string());
                Ok(())
            } else {
                catalog::set_active(id)
            }
        }
        "supprimer" => {
            let mut s = state.lock().unwrap();
            let m = catalog::by_id(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
            if m.id == LOW_PACK.id {
                return Err("le pack low est le repli : il reste installé".into());
            }
            if s.dls.get(id).is_some_and(|d| d.state == "en_cours") {
                return Err("téléchargement en cours : l'annuler d'abord".into());
            }
            if s.active().as_deref() == Some(id) {
                if s.demo {
                    s.demo_active = None;
                } else {
                    catalog::set_active(LOW_PACK.id)?;
                }
            }
            if s.demo {
                s.demo_installed.remove(m.id);
            } else {
                for f in [s.dir.join(m.file), s.dir.join(format!("{}.part", m.file))] {
                    if f.exists() {
                        std::fs::remove_file(&f).map_err(|e| format!("{} : {e}", f.display()))?;
                    }
                }
            }
            s.dls.remove(id);
            Ok(())
        }
        _ => Err(format!("action inconnue : {action}")),
    }
}

/// Icône de la fenêtre : le logo de Chevaliers Translators, RGBA.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn window_icon_rgba(size: u32) -> Vec<u8> {
    crate::brand::logo(size as usize).px
}

// --- Serveur local -------------------------------------------------------------------------

/// Police des titres : celle de WoW (Friz Quadrata), extraite par `fonts` dans le cache.
pub fn game_font() -> Option<Vec<u8>> {
    let root = crate::fonts::cache_root().join("fonts");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.iter().rev().find_map(|d| std::fs::read(d.join("FRIZQT__.TTF")).ok())
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn query_param<'a>(path: &'a str, key: &str) -> Option<&'a str> {
    path.split_once('?')?.1.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

fn handle(mut s: TcpStream, state: &State, token: &str, port: u16) {
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 65536 {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or("/").to_string());
    let headers: HashMap<String, String> = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())))
        .collect();
    // Seulement par l'adresse locale (protection contre les pages web qui viseraient ce port).
    let host_ok = headers
        .get("host")
        .is_some_and(|h| *h == format!("127.0.0.1:{port}") || *h == format!("localhost:{port}"));
    if !host_ok {
        return respond(&mut s, 403, "text/plain; charset=utf-8", "accès refusé".as_bytes());
    }
    {
        let mut st = state.lock().unwrap();
        st.last_hit = Instant::now();
        st.hits += 1;
    }
    let route = path.split('?').next().unwrap_or("/");
    match (method, route) {
        ("GET", "/") | ("GET", "/index.html") => {
            let page = PAGE.replace("__JETON__", token);
            respond(&mut s, 200, "text/html; charset=utf-8", page.as_bytes())
        }
        ("GET", "/logo.png") => respond(&mut s, 200, "image/png", LOGO),
        ("GET", "/banniere.jpg") => respond(&mut s, 200, "image/jpeg", BANNIERE),
        ("GET", "/police.ttf") => match game_font() {
            Some(f) => respond(&mut s, 200, "font/ttf", &f),
            None => respond(&mut s, 404, "text/plain", b""),
        },
        ("GET", "/api/etat") => {
            let st = state.lock().unwrap();
            // Langue de la page (0.16.1) : celle demandée, sinon celle choisie la dernière fois.
            let saved = saved_lang(&st.dir);
            let lang = query_param(&path, "langue").map(str::to_string).or(saved.clone()).unwrap_or_else(|| "fr".into());
            let mut v = state_json(&st, &lang);
            v["langue_choisie"] = json!(saved);
            respond(&mut s, 200, "application/json; charset=utf-8", v.to_string().as_bytes())
        }
        ("POST", r) if r.starts_with("/api/") => {
            if headers.get("x-ct-jeton").map(String::as_str) != Some(token) {
                return respond(&mut s, 403, "application/json", br#"{"erreur":"jeton invalide"}"#);
            }
            let action = &r["/api/".len()..];
            let id = query_param(&path, "id").unwrap_or("");
            match act(state, action, id) {
                Ok(()) => respond(&mut s, 200, "application/json", br#"{"ok":true}"#),
                Err(e) => respond(&mut s, 400, "application/json; charset=utf-8", json!({ "erreur": e }).to_string().as_bytes()),
            }
        }
        _ => respond(&mut s, 404, "text/plain", b""),
    }
}

fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u64(i ^ std::process::id() as u64);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Fenêtre Edge en mode application (profil à part : la fenêtre est un processus à elle).
fn open_window(url: &str) -> Option<std::process::Child> {
    if !cfg!(windows) {
        return None;
    }
    let mut candidates = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join("Microsoft\\Edge\\Application\\msedge.exe"));
        }
    }
    let profile = crate::fonts::cache_root().join("interface");
    for exe in candidates.into_iter().filter(|p| p.is_file()) {
        let child = std::process::Command::new(exe)
            .arg(format!("--app={url}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .args(["--window-size=1060,860", "--no-first-run", "--no-default-browser-check"])
            .spawn();
        if let Ok(c) = child {
            return Some(c);
        }
    }
    // Sans Edge : navigateur par défaut.
    let _ = crate::quiet(&mut std::process::Command::new("cmd")).args(["/C", "start", "", url]).spawn();
    None
}

pub fn run(o: Options) -> Result<bool, String> {
    let listener = TcpListener::bind(("127.0.0.1", o.port)).map_err(|e| format!("port {} : {e}", o.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = new_token();
    let hw = if o.demo { demo_hw() } else { crate::hw::detect() };
    let state: State = Arc::new(Mutex::new(Shared {
        demo: o.demo,
        dir: o.dir.clone(),
        hw,
        dls: HashMap::new(),
        last_hit: Instant::now(),
        hits: 0,
        demo_installed: [LOW_PACK.id].into_iter().collect(),
        demo_active: None,
        demo_runtime: true,
    }));

    // Le pack low et le moteur sont téléchargés d'office.
    {
        let (low_missing, engine_missing) = {
            let s = state.lock().unwrap();
            let o = s.offer();
            (!s.installed(&LOW_PACK) && o.entries[0].eligible(), !s.runtime_ready(o.runtime))
        };
        if engine_missing {
            start_download(&state, ENGINE)?;
        }
        if low_missing {
            start_download(&state, LOW_PACK.id)?;
        }
    }

    // Matériel relu toutes les 5 s (WoW lancé ou fermé, mémoire vidéo qui bouge).
    if !o.demo {
        let st = state.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            let hw = crate::hw::detect();
            st.lock().unwrap().hw = hw;
        });
    }

    {
        let st = state.clone();
        let token = token.clone();
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                let (st, token) = (st.clone(), token.clone());
                std::thread::spawn(move || handle(c, &st, &token, port));
            }
        });
    }

    let url = format!("http://127.0.0.1:{port}/?jeton={token}");
    crate::say!("Chevaliers Translators - modèles de traduction");
    crate::say!("Interface : {url}");
    let opened = Instant::now();
    // Fenêtre de l'application (WebView2) ; à défaut, Edge en mode application.
    #[cfg(windows)]
    if o.window {
        let st = state.clone();
        if let Err(e) = crate::window::show(url.clone(), move || st.lock().unwrap().running()) {
            crate::say!("Fenêtre de l'application impossible ({e}) : ouverture dans Edge.");
        }
    }
    let mut window = if o.window { open_window(&url) } else { None };
    if !o.window {
        crate::say!("(Ouvrir cette adresse dans un navigateur ; Ctrl+C pour arrêter.)");
    }
    // Fin : fenêtre fermée (ou page muette depuis 90 s), et plus aucun téléchargement en cours.
    let mut told = false;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let (running, idle, hits) = {
            let s = state.lock().unwrap();
            (s.running(), s.last_hit.elapsed(), s.hits)
        };
        let closed = match window.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(Some(_)) if opened.elapsed() < Duration::from_secs(3) => {
                    // Edge a passé la main à une fenêtre déjà ouverte : suivre la page à la place.
                    window = None;
                    false
                }
                Ok(Some(_)) => true,
                _ => false,
            },
            None => o.window && hits > 0 && idle > Duration::from_secs(90),
        };
        if closed && !running {
            crate::say!("Fenêtre fermée.");
            return Ok(true);
        }
        if closed && running && !told {
            told = true;
            crate::say!("Fenêtre fermée : les téléchargements en cours continuent ici (Ctrl+C pour les arrêter ; ils reprendront au prochain lancement).");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, path: &str, host: &str, token: Option<&str>, method: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let tok = token.map_or(String::new(), |t| format!("X-CT-Jeton: {t}\r\n"));
        write!(s, "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{tok}Content-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        let mut r = String::new();
        s.read_to_string(&mut r).unwrap();
        let code = r.split_whitespace().nth(1).unwrap().parse().unwrap();
        (code, r.split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default())
    }

    #[test]
    fn window_icon_is_the_logo() {
        let px = window_icon_rgba(64);
        assert_eq!(px.len(), 64 * 64 * 4);
        assert_eq!(px[3], 0, "coin transparent");
        let c = &px[(32 * 64 + 32) * 4..][..4];
        assert!(c[3] >= 250, "médaillon opaque : {c:?}");
    }

    #[test]
    fn demo_server_states_and_actions() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!("ct-ui-test-{}", std::process::id())).join("modeles");
        std::fs::create_dir_all(&dir).unwrap();
        let state: State = Arc::new(Mutex::new(Shared {
            demo: true,
            dir: dir.clone(),
            hw: demo_hw(),
            dls: HashMap::new(),
            last_hit: Instant::now(),
            hits: 0,
            demo_installed: [LOW_PACK.id].into_iter().collect(),
            demo_active: None,
            demo_runtime: true,
        }));
        let st = state.clone();
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                let st = st.clone();
                std::thread::spawn(move || handle(c, &st, "jeton", port));
            }
        });
        let host = format!("127.0.0.1:{port}");
        // Page avec le jeton ; autre hôte refusé.
        let (code, page) = get(port, "/?jeton=jeton", &host, None, "GET");
        assert_eq!(code, 200);
        assert!(page.contains("const JETON = \"jeton\""));
        assert_eq!(get(port, "/api/etat", "evil.example:80", None, "GET").0, 403);
        // État : 4B = HIGH (7B a fait planter le jeu sur la carte), low actif par défaut.
        let (_, body) = get(port, "/api/etat", &host, None, "GET");
        let v: Value = serde_json::from_str(&body).unwrap();
        let ids: Vec<&str> = v["modeles"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["eurollm-1.7b", "translategemma-4b", "salamandrata-7b"]);
        assert_eq!(v["modeles"][1]["high"], true);
        // Estimation : 4B conseillé et premier de la chaîne, sur la carte puis sur le
        // processeur, puis le pack low.
        assert_eq!(v["modeles"][1]["conseille"], true);
        assert_eq!(v["modeles"][1]["rang"], 1);
        assert_eq!(v["modeles"][1]["estimation"]["niveau"], "sûr");
        assert_eq!(v["estimation"]["chaine"][1], "TranslateGemma 4B sur le processeur");
        assert!(v["estimation"]["chaine"][2].as_str().unwrap().starts_with("EuroLLM 1.7B"));
        assert_eq!(v["modeles"][2]["conseille"], false);
        assert_eq!(v["modeles"][0]["actif"], true);
        assert_eq!(v["modeles"][2]["lieu"], Value::Null);
        assert!(v["modeles"][2]["raison"].as_str().unwrap().contains("a fait planter le jeu"));
        // Action sans jeton : refusée ; modèle non proposé : erreur expliquée.
        assert_eq!(get(port, "/api/telecharger?id=translategemma-4b", &host, None, "POST").0, 403);
        let (code, err) = get(port, "/api/telecharger?id=salamandrata-7b", &host, Some("jeton"), "POST");
        assert_eq!(code, 400);
        assert!(err.contains("n'est pas proposé"));
        // Activer avant téléchargement : refusé ; télécharger puis activer.
        assert_eq!(get(port, "/api/activer?id=translategemma-4b", &host, Some("jeton"), "POST").0, 400);
        assert_eq!(get(port, "/api/telecharger?id=translategemma-4b", &host, Some("jeton"), "POST").0, 200);
        let t0 = Instant::now();
        while !state.lock().unwrap().demo_installed.contains("translategemma-4b") {
            assert!(t0.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(get(port, "/api/activer?id=translategemma-4b", &host, Some("jeton"), "POST").0, 200);
        let (_, body) = get(port, "/api/etat", &host, None, "GET");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["modeles"][1]["actif"], true);
        assert_eq!(v["modeles"][0]["actif"], false);
        // Supprimer le modèle actif : retour au pack low ; le pack low ne se supprime pas.
        assert_eq!(get(port, "/api/supprimer?id=translategemma-4b", &host, Some("jeton"), "POST").0, 200);
        assert_eq!(get(port, "/api/supprimer?id=eurollm-1.7b", &host, Some("jeton"), "POST").0, 400);
        let (_, body) = get(port, "/api/etat", &host, None, "GET");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((v["modeles"][0]["actif"].as_bool(), v["modeles"][1]["installe"].as_bool()), (Some(true), Some(false)));
        // Langue de la page (0.16.1) : gardée par l'application, reprise sans paramètre ; la
        // demande de la page passe avant ; langue inconnue refusée.
        assert_eq!(v["langue_choisie"], Value::Null);
        assert_eq!(get(port, "/api/langue?id=xx", &host, Some("jeton"), "POST").0, 400);
        assert_eq!(get(port, "/api/langue?id=de", &host, Some("jeton"), "POST").0, 200);
        let (_, body) = get(port, "/api/etat", &host, None, "GET");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((v["langue"].as_str(), v["langue_choisie"].as_str()), (Some("de"), Some("de")));
        assert_eq!(v["modeles"][0]["estimation"]["niveau_code"].as_str().map(|c| ["sure", "likely", "borderline", "no"].contains(&c)), Some(true));
        let (_, body) = get(port, "/api/etat?langue=ru", &host, None, "GET");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["langue"], "ru");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
