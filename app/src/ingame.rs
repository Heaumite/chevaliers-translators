//! Tests lancés depuis WoW : l'application tourne en arrière-plan, lit les demandes de l'addon
//! dans son bandeau, lance les tests et affiche leur avancement et leur verdict par-dessus le
//! jeu. Le joueur ne quitte jamais WoW : le jeu reste au premier plan pendant toute la mesure.
//!
//! `/chevaliers essai` lance la série complète, du plus prudent au plus lourd (pack low sur le
//! processeur, pack low sur la carte, 4B sur la carte, 4B sur le processeur, 7B), saute ce qui
//! n'a plus de sens (modèle absent, modèle plus lourd après un échec de charge sur la carte),
//! affiche le bilan et ferme l'application.

// Utilisé par la boucle Windows ; ailleurs, seulement par les tests.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::bench;
use crate::catalog::{Model, Verdict, LOW_PACK, ON_DEMAND};
use crate::mirror::Rgb;

/// Un test de la série.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub model: &'static Model,
    pub cpu: bool,
    pub low: bool,
}

impl Step {
    pub fn label(&self) -> String {
        format!("{} {}", self.model.name, if self.cpu { "sur le processeur" } else { "sur la carte graphique" })
    }
}

/// Demande reçue de l'addon.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Test(Step),
    Series,
    Stop,
    Close,
    Unknown(String),
}

/// Série complète, du plus prudent au plus lourd.
pub fn full_series() -> Vec<Step> {
    vec![
        Step { model: &LOW_PACK, cpu: true, low: true },
        Step { model: &LOW_PACK, cpu: false, low: true },
        Step { model: &ON_DEMAND[0], cpu: false, low: false },
        // Repli « plante sur la carte, pas sur le processeur » : le 4B sur le processeur.
        Step { model: &ON_DEMAND[0], cpu: true, low: false },
        Step { model: &ON_DEMAND[1], cpu: false, low: false },
    ]
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub step: Step,
    /// Verdict du test ; `None` : sauté ou impossible.
    pub verdict: Option<Verdict>,
    pub recorded: bool,
    pub text: String,
}

/// Déroulement d'une série.
pub struct Series {
    steps: Vec<Step>,
    next: usize,
    pub results: Vec<Outcome>,
    gpu_failed: bool,
    pub stopped: bool,
}

impl Series {
    pub fn new(steps: Vec<Step>) -> Series {
        Series { steps, next: 0, results: Vec::new(), gpu_failed: false, stopped: false }
    }

    pub fn total(&self) -> usize {
        self.steps.len()
    }

    /// Prochain test à lancer : les modèles absents sont sautés, et après un échec sur la
    /// carte graphique, les modèles plus lourds sur la carte aussi.
    pub fn next_step(&mut self, available: &dyn Fn(&Model) -> bool) -> Option<(usize, Step)> {
        while self.next < self.steps.len() && !self.stopped {
            let i = self.next;
            let st = self.steps[i];
            self.next += 1;
            let skip = if !available(st.model) {
                Some("pas téléchargé : sauté")
            } else if !st.cpu && self.gpu_failed {
                Some("sauté : un modèle plus léger n'a pas tenu sur la carte")
            } else {
                None
            };
            match skip {
                Some(why) => self.results.push(Outcome { step: st, verdict: None, recorded: false, text: why.into() }),
                None => return Some((i + 1, st)),
            }
        }
        None
    }

    pub fn record(&mut self, step: Step, verdict: Option<Verdict>, recorded: bool, text: String) {
        // Un modèle plus lourd n'est sauté qu'après un échec de charge sur la carte
        // (plantage, mémoire, images par seconde, à-coups). Un échec de latence seule ne dit
        // rien des plus gros : le 7B traduit plus vite que le 4B sur ce PC.
        let latency_only = verdict == Some(Verdict::Heavy)
            && text.contains("latence")
            && !["images par seconde", "à-coups", "ralentit"].iter().any(|k| text.contains(k));
        // Mesures incomplètes (WoW à l'arrière-plan) : pas un échec, rien n'est noté.
        let incomplete = !recorded && verdict != Some(Verdict::Crash);
        if !step.cpu && verdict.is_some() && verdict != Some(Verdict::Ok) && !latency_only && !incomplete {
            self.gpu_failed = true;
        }
        self.results.push(Outcome { step, verdict, recorded, text });
    }

    /// Lignes du bilan, avec leur couleur.
    pub fn summary(&self) -> Vec<(String, Rgb)> {
        self.results
            .iter()
            .map(|o| (format!("{} : {}", o.step.label(), o.text), crate::panel::verdict_color(o.verdict, o.recorded)))
            .collect()
    }
}

/// Sépare la demande de son numéro (« essai 4b\t12345 »).
pub fn split_id(raw: &str) -> (&str, Option<&str>) {
    match raw.split_once('\t') {
        Some((t, id)) => (t, Some(id.trim())),
        None => (raw, None),
    }
}

pub fn parse_request(text: &str) -> Request {
    let step = |model: &'static Model, cpu: bool, low: bool| Request::Test(Step { model, cpu, low });
    let t = text.trim().to_lowercase();
    let arg = t
        .strip_prefix("essai")
        .or_else(|| t.strip_prefix("test"))
        .map(str::trim)
        .unwrap_or(&t);
    match arg {
        "" | "tout" | "serie" | "série" | "all" => Request::Series,
        "low" => step(&LOW_PACK, false, true),
        "low-cpu" | "lowcpu" => step(&LOW_PACK, true, true),
        "4b" => step(&ON_DEMAND[0], false, false),
        "4b-cpu" => step(&ON_DEMAND[0], true, false),
        "7b" => step(&ON_DEMAND[1], false, false),
        "stop" | "arret" | "arrêt" => Request::Stop,
        "fermer" | "close" | "ok" => Request::Close,
        other => Request::Unknown(other.to_string()),
    }
}

fn options(dir: &std::path::Path, out: &std::path::Path, m: &Model, cpu: bool, low: bool) -> bench::Options {
    bench::Options {
        llama: None,
        runtime: "vulkan".into(),
        model: dir.join(m.file),
        cpu,
        court: true,
        low,
        fps: true,
        forcer: false,
        port: 8089,
        out: out.to_path_buf(),
        tuning: crate::llama::Tuning::gentle(),
        pace: None,
        threads: None,
        regulate: true,
        bursts: (!cpu).then(crate::burst::Bursts::gentle),
        status: None,
        stop: None,
        from_game: true,
    }
}

/// Boucle de l'application : demandes de WoW, tests, panneau par-dessus le jeu (Windows).
#[cfg(windows)]
pub fn run(dir: std::path::PathBuf, out: std::path::PathBuf) -> Result<bool, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use crate::bench::Status;
    use crate::overlay_win::Overlay;
    use crate::panel::{self, Panel};

    /// Fenêtre du panneau (identifiant hors de ceux des fenêtres de chat).
    const PANEL: u8 = 250;
    type Job = (JoinHandle<Result<bool, String>>, Arc<AtomicBool>);

    let fonts = panel::Fonts::load();
    let mut overlay = Overlay::new()?;
    // Lecture du bandeau dans un fil à part, toutes les 5 ms : une trame ne reste que 50 ms
    // à l'écran.
    let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
    let (dims_tx, dims_rx) = mpsc::channel::<Result<(i32, i32), String>>();
    std::thread::spawn(move || {
        let mut cap = match crate::capture_win::Capturer::new(0, 48) {
            Ok(c) => {
                let _ = dims_tx.send(Ok((c.screen_w, c.screen_h)));
                c
            }
            Err(e) => {
                let _ = dims_tx.send(Err(e));
                return;
            }
        };
        let mut last_seq: Option<u16> = None;
        loop {
            if let Ok(img) = cap.grab() {
                if let Ok(d) = crate::decode::decode(&img) {
                    if d.frame.ftype == crate::proto::FRAME_CMD && last_seq != Some(d.frame.seq) {
                        last_seq = Some(d.frame.seq);
                        if cmd_tx.send(String::from_utf8_lossy(&d.frame.payload).to_string()).is_err() {
                            return;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let (sw, sh) = dims_rx.recv().map_err(|e| e.to_string())??;
    let width = ((sw as f32 * 0.24) as usize).clamp(460, 780);
    let px = (sh as f32 / 70.0).clamp(15.0, 24.0);
    crate::say!("Tests en jeu : l'application attend les demandes de WoW (Ctrl+C pour arrêter).");
    crate::say!("Dans le chat de WoW : /chevaliers essai (série complète, l'application se ferme à la fin)");
    crate::say!("  ou un seul test : /chevaliers essai low | low-cpu | 4b | 7b ; /chevaliers essai stop ; /chevaliers essai fermer");
    crate::say!("Résultats : {}", out.display());

    let available = |m: &Model| crate::catalog::installed_in(&dir, m);
    let launch = |st: Step, title: String, tx: &mpsc::Sender<Status>| -> (Panel, Job) {
        let stop = Arc::new(AtomicBool::new(false));
        let mut o = options(&dir, &out, st.model, st.cpu, st.low);
        o.status = Some(tx.clone());
        o.stop = Some(stop.clone());
        crate::say!("Test : {}", st.label());
        (Panel::start(&title), (std::thread::spawn(move || bench::run(&o)), stop))
    };

    let mut panel = Some(Panel::ready());
    // Le panneau d'accueil reste 45 s à partir de sa première apparition au-dessus de WoW.
    let mut hide_at: Option<Instant> = None;
    let mut ready_pending = true;
    let mut dirty = true;
    let mut shown = false;
    let mut seen_ids: Vec<String> = Vec::new();
    let (tx, rx) = mpsc::channel::<Status>();
    let mut job: Option<Job> = None;
    let mut current: Option<Step> = None;
    let mut last_done: Option<(Vec<String>, Verdict, bool, bool)> = None;
    let mut series: Option<Series> = None;
    let mut next_at: Option<Instant> = None;
    let mut exit_at: Option<Instant> = None;
    loop {
        // 1. Demandes de l'addon (envoyées trois fois avec le même numéro).
        while let Ok(raw) = cmd_rx.try_recv() {
            let (text, id) = split_id(&raw);
            if let Some(id) = id {
                if seen_ids.iter().any(|s| s == id) {
                    continue;
                }
                seen_ids.push(id.to_string());
                if seen_ids.len() > 50 {
                    seen_ids.remove(0);
                }
            }
            ready_pending = false;
            crate::say!("Demande de WoW : {text}");
            let busy = job.is_some() || series.is_some();
            match parse_request(text) {
                Request::Test(_) | Request::Series if busy => {
                    if let Some(p) = panel.as_mut() {
                        p.notes.push("Un test est déjà en cours : /chevaliers essai stop pour l'arrêter.".into());
                    }
                }
                Request::Test(st) => {
                    let (p, j) = launch(st, format!("Test en jeu : {}", st.label()), &tx);
                    panel = Some(p);
                    job = Some(j);
                    current = Some(st);
                    hide_at = None;
                }
                Request::Series => {
                    let mut p = Panel::ready();
                    p.title = "Série de tests".into();
                    p.phase = "Départ dans 5 s : ne bouge plus jusqu'au bilan (environ 15 min)".into();
                    p.footer = "Reste dans WoW jusqu'au bilan (environ 20 min) : sans WoW au premier plan, pas de mesure. /chevaliers essai stop arrête la série.".into();
                    p.done = false;
                    panel = Some(p);
                    series = Some(Series::new(full_series()));
                    next_at = Some(Instant::now() + Duration::from_secs(5));
                    hide_at = None;
                }
                Request::Stop => {
                    if let Some((_, stop)) = &job {
                        stop.store(true, Ordering::Relaxed);
                    }
                    if let Some(se) = series.as_mut() {
                        se.stopped = true;
                    }
                }
                Request::Close => {
                    if job.is_none() && series.is_none() {
                        panel = None;
                    }
                }
                Request::Unknown(w) => {
                    if let Some(p) = panel.as_mut() {
                        p.error = Some(format!("Test inconnu : « {w} »"));
                    } else {
                        let mut p = Panel::ready();
                        p.error = Some(format!("Test inconnu : « {w} »"));
                        panel = Some(p);
                        hide_at = Some(Instant::now() + Duration::from_secs(30));
                    }
                }
            }
            dirty = true;
        }
        // 2. Avancement du test.
        while let Ok(s) = rx.try_recv() {
            if let Status::Done { lines, verdict, recorded, aborted } = &s {
                last_done = Some((lines.clone(), *verdict, *recorded, *aborted));
            }
            if let Some(p) = panel.as_mut() {
                p.apply(s);
                dirty = true;
            }
        }
        if job.as_ref().is_some_and(|(h, _)| h.is_finished()) {
            let (h, _) = job.take().unwrap();
            let r = h.join().unwrap_or_else(|_| Err("le test s'est arrêté brutalement".into()));
            let done = last_done.take();
            if let (Err(e), Some(p)) = (&r, panel.as_mut()) {
                if !p.done {
                    p.fail(e);
                }
            }
            let st = current.take();
            match (series.as_mut(), st) {
                (Some(se), Some(st)) => {
                    let (verdict, recorded, text) = match (&r, done) {
                        (Err(e), _) => (None, false, format!("impossible ({e})")),
                        (Ok(_), Some((lines, v, rec, aborted))) => {
                            if aborted {
                                se.stopped = true;
                            }
                            let line = lines
                                .iter()
                                .rev()
                                .find(|l| l.starts_with("Verdict") || l.starts_with("Test arrêté"))
                                .cloned()
                                .unwrap_or_default();
                            let short = line.split_once(" : ").map_or(line.clone(), |(_, b)| b.to_string());
                            (if aborted { None } else { Some(v) }, rec, short)
                        }
                        (Ok(_), None) => (None, false, "pas de bilan".into()),
                    };
                    crate::say!("  {} : {text}", st.label());
                    se.record(st, verdict, recorded, text);
                    // WoW fermé (plantage) : la série ne peut pas continuer.
                    if crate::hw::wow_pids().is_empty() {
                        se.stopped = true;
                    }
                    next_at = Some(Instant::now() + Duration::from_secs(10));
                    if let Some(p) = panel.as_mut() {
                        p.footer = "Test suivant dans 10 s. Reste dans WoW. /chevaliers essai stop arrête la série.".into();
                    }
                }
                _ => hide_at = Some(Instant::now() + Duration::from_secs(180)),
            }
            dirty = true;
        }
        // 3. Série : test suivant, ou bilan et fermeture.
        if job.is_none() && next_at.is_some_and(|t| Instant::now() >= t) {
            next_at = None;
            if let Some(se) = series.as_mut() {
                match se.next_step(&available) {
                    Some((i, st)) => {
                        let total = se.total();
                        let (p, j) = launch(st, format!("Série {i}/{total} : {}", st.label()), &tx);
                        panel = Some(p);
                        job = Some(j);
                        current = Some(st);
                    }
                    None => {
                        let mut p = Panel::ready();
                        p.title = if se.stopped { "Série de tests arrêtée".into() } else { "Série de tests terminée".into() };
                        p.phase = "Bilan :".into();
                        p.colored = se.summary();
                        p.footer = format!("L'application se ferme dans 60 s. Détails : {}", out.display());
                        panel = Some(p);
                        let mut txt = String::from("Série de tests en jeu\n");
                        for (l, _) in se.summary() {
                            txt.push_str(&format!("  {l}\n"));
                        }
                        crate::say!("{txt}");
                        let _ = std::fs::create_dir_all(&out);
                        let _ = std::fs::write(out.join(format!("serie-{}.txt", crate::catalog::now_secs())), &txt);
                        series = None;
                        exit_at = Some(Instant::now() + Duration::from_secs(60));
                    }
                }
                dirty = true;
            }
        }
        if exit_at.is_some_and(|t| Instant::now() >= t) {
            overlay.hide(PANEL);
            crate::say!("Série terminée : fermeture de l'application.");
            return Ok(true);
        }
        if hide_at.is_some_and(|t| Instant::now() > t) {
            panel = None;
            hide_at = None;
        }
        // 4. Affichage par-dessus WoW (seulement quand WoW est devant).
        match (&panel, Overlay::wow_in_front()) {
            (Some(p), true) => {
                if dirty || !shown {
                    // En haut à droite de la zone de jeu de WoW (plein écran ou fenêtré).
                    let (l, top, right, _) = crate::capture_win::wow_client_rect().unwrap_or((0, 0, sw, sh));
                    let width = (((right - l) as f32 * 0.3) as usize).clamp(420, width);
                    let cv = panel::render(&fonts, &p.rows(), width, px);
                    let margin = (px * 1.5) as i32;
                    let left = right - cv.w as i32 - margin;
                    let bottom = sh - (top + margin) - cv.h as i32;
                    if overlay.show(PANEL, left, bottom, &cv, &[]).is_ok() {
                        shown = true;
                        dirty = false;
                        if ready_pending && hide_at.is_none() {
                            hide_at = Some(Instant::now() + Duration::from_secs(45));
                        }
                    }
                }
            }
            _ => {
                if shown {
                    overlay.hide(PANEL);
                    shown = false;
                }
            }
        }
        overlay.pump();
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_from_the_game() {
        assert_eq!(parse_request("essai 7b"), Request::Test(Step { model: &ON_DEMAND[1], cpu: false, low: false }));
        assert_eq!(parse_request("test LOW-CPU"), Request::Test(Step { model: &LOW_PACK, cpu: true, low: true }));
        assert_eq!(parse_request("essai"), Request::Series);
        assert_eq!(parse_request("essai tout"), Request::Series);
        assert_eq!(parse_request("test stop"), Request::Stop);
        assert_eq!(parse_request("test fermer"), Request::Close);
        assert_eq!(parse_request("test 12b"), Request::Unknown("12b".into()));
        assert_eq!(split_id("essai 4b\t1234567"), ("essai 4b", Some("1234567")));
        assert_eq!(split_id("essai 4b"), ("essai 4b", None));
        let o = options(std::path::Path::new("m"), std::path::Path::new("r"), &ON_DEMAND[0], false, false);
        assert!(o.fps && o.court && o.from_game && o.bursts.is_some());
        assert_eq!(o.model, std::path::Path::new("m").join("translategemma-4b-it.Q4_K_M.gguf"));
    }

    #[test]
    fn series_runs_by_itself_and_skips_what_no_longer_makes_sense() {
        // Tout est installé ; le 4B échoue sur la carte : le 7B, plus lourd, est sauté.
        let mut se = Series::new(full_series());
        let all = |_: &Model| true;
        let (i, st) = se.next_step(&all).unwrap();
        assert_eq!((i, st.cpu, st.model.id), (1, true, "eurollm-1.7b"));
        se.record(st, Some(Verdict::Ok), true, "compatible".into());
        let (i, st) = se.next_step(&all).unwrap();
        assert_eq!((i, st.cpu), (2, false));
        se.record(st, Some(Verdict::Ok), true, "compatible".into());
        let (_, st) = se.next_step(&all).unwrap();
        assert_eq!(st.model.id, "translategemma-4b");
        se.record(st, Some(Verdict::Crash), true, "a fait planter le jeu".into());
        // Le 4B sur le processeur est tout de même essayé : la carte ne le concerne pas.
        let (_, st) = se.next_step(&all).unwrap();
        assert_eq!((st.model.id, st.cpu), ("translategemma-4b", true));
        se.record(st, Some(Verdict::Ok), true, "compatible".into());
        assert!(se.next_step(&all).is_none());
        let sum = se.summary();
        assert_eq!(sum.len(), 5);
        assert!(sum[4].0.starts_with("SalamandraTA 7B v3 sur la carte graphique : sauté"));
        // Le 4B trop lent seulement (latence) : le 7B est tout de même essayé.
        let mut se = Series::new(full_series());
        for _ in 0..2 {
            let (_, st) = se.next_step(&all).unwrap();
            se.record(st, Some(Verdict::Ok), true, "compatible".into());
        }
        let (_, st) = se.next_step(&all).unwrap();
        se.record(st, Some(Verdict::Heavy), true, "trop lourd pour ce PC avec ces réglages - latence médiane 1.71 s (seuil 1.5 s)".into());
        let (_, st) = se.next_step(&all).unwrap();
        se.record(st, Some(Verdict::Heavy), true, "trop lourd pour ce PC avec ces réglages - latence médiane 3.4 s (seuil 3 s)".into());
        assert_eq!(se.next_step(&all).unwrap().1.model.id, "salamandrata-7b");
        // Mesures incomplètes (WoW à l'arrière-plan) sur le 4B : pas un échec, le 7B est essayé.
        let mut se = Series::new(full_series());
        for _ in 0..2 {
            let (_, st) = se.next_step(&all).unwrap();
            se.record(st, Some(Verdict::Ok), true, "compatible".into());
        }
        let (_, st) = se.next_step(&all).unwrap();
        se.record(st, Some(Verdict::Heavy), false, "aucun, mesures en jeu incomplètes (baisse moyenne d'images par seconde 5.2 %)".into());
        let (_, st) = se.next_step(&all).unwrap();
        se.record(st, Some(Verdict::Ok), true, "compatible".into());
        assert_eq!(se.next_step(&all).unwrap().1.model.id, "salamandrata-7b");
        // 7B absent : sauté sans échec ; arrêt demandé : plus rien.
        let mut se = Series::new(full_series());
        let no7 = |m: &Model| m.id != "salamandrata-7b";
        for _ in 0..4 {
            let (_, st) = se.next_step(&no7).unwrap();
            se.record(st, Some(Verdict::Ok), true, "compatible".into());
        }
        assert!(se.next_step(&no7).is_none());
        assert!(se.summary()[4].0.ends_with("pas téléchargé : sauté"));
        let mut se = Series::new(full_series());
        se.stopped = true;
        assert!(se.next_step(&all).is_none());
    }
}
