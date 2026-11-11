//! ct-outils : outils de développement de Chevaliers Translators (tests de la phase 0, banc des
//! modèles, installation à la main). L'application des joueurs est `ChevaliersTranslators.exe`.
//!
//!   selftest                      auto-test du canal pixel, sans le jeu (WSL ou Windows)
//!   live                          décodeur en direct, chat miroir et traduction (Windows uniquement)
//!   install [--wow <dossier>]     installe l'addon dans toutes les versions du jeu
//!   addon                         état de l'addon dans chaque version du jeu
//!   fonts --wow <dossier>         extrait les polices du chat des données du jeu (S5a)
//!   materiel                      détecte le matériel et propose un niveau de traduction (S4a)
//!   bench --modele <f> [--fps]    comparatif des modèles et test en jeu (S4b)
//!   estimation [--wow <dossier>]  modèle conseillé, chaîne de repli et téléchargements, sans test
//!   installation [--telecharger]  modèles : annonce, modèles conseillés et moteur, modèle sur demande
//!   interface                     fenêtre de gestion des modèles
//!   diagnostic [--heures N]       réinitialisations du pilote graphique et plantages récents
//!   essai --modele <f> --verdict  note à la main le résultat d'un test en jeu
//!   decode-raw <fichier> <l> <h>  décode une image BGRA brute (harnais de test)



use ct::*;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage :
  ct-outils selftest [--frames N]
  ct-outils live [--width PX] [--height PX] [--poll MS] [--wow <dossier> --flavor <_x_>] [--demo-trad] [--dx PX] [--dy PX]
                [--langue xx] [--dossier <dossier des modèles>] [--sans-mes-messages] [--sans-traduction] [--port-traduction 8090]
  ct-outils install [--wow <dossier World of Warcraft>] [--flavor _forever_] [--interface N]
  ct-outils addon [--wow <dossier World of Warcraft>]          (état de l'addon dans chaque version du jeu)
  ct-outils fonts --wow <dossier> --flavor <_x_> [--locale frFR] [--file \"Fonts\\X.TTF\"]... [--fdid N]... [--out <dossier>]
  ct-outils materiel
  ct-outils estimation [--wow <dossier World of Warcraft> --flavor <_x_>]
  ct-outils installation [--telecharger] [--seulement-low] [--modele <id>] [--moteur vulkan|hip-radeon|cpu] [--dossier <dossier des modèles>]
  ct-outils interface [--dossier <dossier des modèles>] [--sans-fenetre] [--demo] [--port N]
  ct-outils bench --modele <fichier.gguf> [--fps] [--court] [--low] [--cpu] [--fils N] [--rythme S]
                 [--moteur vulkan|hip-radeon] [--llama <dossier>] [--origine] [--priorite-gpu normale|basse|minimale]
                 [--paquets N] [--lots N] [--file-graphique] [--couches N] [--rafales N] [--pause MS]
                 [--sans-rafales] [--sans-regulateur] [--forcer]
                 [--port 8089] [--sortie <dossier>]
  ct-outils tests-en-jeu [--dossier <dossier des modèles>] [--sortie <dossier>]   (tests lancés depuis WoW : /chevaliers essai 4b)
  ct-outils diagnostic [--wow <dossier> --flavor <_x_>] [--heures 12]
  ct-outils essai --modele <fichier.gguf> --verdict ok|lourd|plantage|memoire [--sur carte|processeur] [--note \"...\"]
  ct-outils decode-raw <fichier.bgra> <largeur> <hauteur>";

struct Args(Vec<String>);

impl Args {
    fn value(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .position(|a| a == name)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn values(&self, name: &str) -> Vec<String> {
        self.0
            .windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].clone())
            .collect()
    }
    fn flag(&self, name: &str) -> bool {
        self.0.iter().any(|a| a == name)
    }
    fn font_source(&self) -> live::FontSource {
        if let Some(f) = self.value("--police") {
            return live::FontSource::File(PathBuf::from(f));
        }
        match (self.value("--wow"), self.value("--flavor")) {
            (Some(w), Some(f)) => live::FontSource::Game {
                wow: PathBuf::from(w),
                flavor: f.to_string(),
            },
            _ => live::FontSource::None,
        }
    }
    fn render_opts(&self) -> Result<mirror::RenderOpts, String> {
        Ok(mirror::RenderOpts {
            // Calage du 08/11/2026 (Forever, Arial Narrow 16 px) : texte du miroir 1 px à gauche.
            dx: self.num("--dx", 1.0)?,
            dy: self.num("--dy", 0.0)?,
            plate: None,
        })
    }
    fn num<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.value(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| format!("valeur invalide pour {name} : {v}")),
        }
    }
    /// Réglages du serveur sur la carte graphique : mode doux, sauf --origine.
    fn tuning(&self) -> Result<llama::Tuning, String> {
        let mut t = if self.flag("--origine") { llama::Tuning::raw() } else { llama::Tuning::gentle() };
        if let Some(p) = self.value("--priorite-gpu") {
            t.priority = llama::GpuPriority::parse(p).ok_or_else(|| format!("--priorite-gpu normale|basse|minimale, pas « {p} »"))?;
        }
        if let Some(n) = self.value("--paquets") {
            let n: u32 = n.parse().map_err(|_| format!("--paquets invalide : {n}"))?;
            t.nodes_per_submit = (n > 0).then_some(n);
        }
        if let Some(n) = self.value("--lots") {
            let n: u32 = n.parse().map_err(|_| format!("--lots invalide : {n}"))?;
            t.ubatch = (n > 0).then_some(n);
        }
        if self.flag("--file-graphique") {
            t.graphics_queue = true;
        }
        if let Some(n) = self.value("--couches") {
            t.gpu_layers = Some(n.parse().map_err(|_| format!("--couches invalide : {n}"))?);
        }
        Ok(t)
    }
    /// Rafales : par défaut sur la carte graphique (16 jetons, 30 ms), pas sur le processeur.
    fn bursts(&self) -> Result<Option<burst::Bursts>, String> {
        if self.flag("--sans-rafales") {
            return Ok(None);
        }
        let given = self.value("--rafales").is_some() || self.value("--pause").is_some();
        if self.flag("--cpu") && !given {
            return Ok(None);
        }
        let mut b = burst::Bursts::gentle();
        if let Some(n) = self.value("--rafales") {
            b.tokens = n.parse().map_err(|_| format!("--rafales invalide : {n}"))?;
            if b.tokens == 0 {
                return Ok(None);
            }
        }
        if let Some(p) = self.value("--pause") {
            b.pause_ms = p.parse().map_err(|_| format!("--pause invalide : {p}"))?;
        }
        Ok(Some(b))
    }
    fn addons(&self) -> Result<PathBuf, String> {
        if let Some(a) = self.value("--addons") {
            return Ok(PathBuf::from(a));
        }
        match (self.value("--wow"), self.value("--flavor")) {
            (Some(w), Some(f)) => Ok(install::addons_dir(&PathBuf::from(w), f)),
            _ => Err("préciser --addons <dossier> ou --wow <dossier> --flavor <_x_>".into()),
        }
    }
}

fn run() -> Result<bool, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let cmd = raw.first().cloned().unwrap_or_default();
    let args = Args(raw);
    match cmd.as_str() {
        "selftest" => Ok(selftest::run(args.num("--frames", 3000)?)),
        "live" => {
            // Traduction réelle, sauf --sans-traduction (ou la démonstration --demo-trad).
            let tr = if args.flag("--sans-traduction") || args.flag("--demo-trad") {
                None
            } else {
                let wow = args.value("--wow").map(PathBuf::from);
                let flavor = args.value("--flavor").map(String::from);
                let locale = match (&wow, &flavor) {
                    (Some(w), Some(f)) => fonts::client_locale_of(w, f),
                    _ => "enUS".into(),
                };
                let client = match args.value("--langue") {
                    Some(l) => lang::LANGS
                        .iter()
                        .find(|x| x.eq_ignore_ascii_case(l))
                        .copied()
                        .ok_or_else(|| format!("--langue {l} : langue inconnue (attendu : {})", lang::LANGS.join(", ")))?,
                    None => lang::from_locale(&locale).unwrap_or("en"),
                };
                Some((
                    engine::Config {
                        wow: wow.zip(flavor).map(|(w, f)| w.join(f)),
                        models: args.value("--dossier").map(PathBuf::from).unwrap_or_else(catalog::models_dir),
                        port: args.num("--port-traduction", 8090)?,
                        log: engine::log_path(),
                        phrases: Some(engine::phrases_path()),
                    },
                    client,
                    // 0.11 : les messages du joueur sont traduits aussi (--mes-messages : ancien nom).
                    !args.flag("--sans-mes-messages"),
                ))
            };
            if let (Some(w), Some(f)) = (args.value("--wow"), args.value("--flavor")) {
                let me = env!("CARGO_PKG_VERSION");
                match install::addon_version(std::path::Path::new(w), f) {
                    Some(v) if v == me => {}
                    Some(v) => println!("ATTENTION : addon {v} dans {f}, l'application est en {me} : relancer « install », puis /reload en jeu."),
                    None => println!("ATTENTION : addon absent de {f} : lancer « install », puis relancer WoW."),
                }
            }
            live::run(
                args.num("--width", 0)?,
                args.num("--height", 48)?,
                args.num("--poll", 4)?,
                args.addons().ok(),
                args.font_source(),
                args.flag("--demo-trad"),
                args.render_opts()?,
                tr,
                &mut || true,
            )
            .map(|_| true)
        }
        "feed" => {
            // Harnais : décode une suite d'images BGRA comme une capture en direct.
            let w: usize = args.num("--w", 0)?;
            let h: usize = args.num("--h", 0)?;
            let mut live = live::Live::new(args.addons().ok());
            let out = args.value("--miroir-out").map(PathBuf::from);
            if out.is_some() {
                live.enable_mirror(args.font_source(), args.flag("--demo-trad"), args.render_opts()?);
            }
            for file in args.0.iter().skip(1).filter(|a| a.ends_with(".bgra")) {
                let data = std::fs::read(file).map_err(|e| format!("{file} : {e}"))?;
                if data.len() != w * h * 4 {
                    return Err(format!("{file} : taille inattendue"));
                }
                live.feed(&decode::Image {
                    w,
                    h,
                    stride: w * 4,
                    bgra: &data,
                });
            }
            if live.last_alive.is_some() {
                println!("[miroir] signe de vie de l'addon reçu");
            }
            if live.waiting_at.is_some() {
                println!("[présence] l'addon attend l'application (/reload)");
            }
            for (id, text) in live.sends_due(std::time::Instant::now(), false) {
                println!("[S3] collage prêt pour l'envoi {id} : {text}");
            }
            if let Some(dir) = out {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                for u in live.mirror_tick(std::time::Instant::now() + mirror::GRACE) {
                    let file = dir.join(format!("miroir_{}.bmp", u.frame));
                    match u.canvas {
                        Some(cv) => {
                            std::fs::write(&file, cv.to_bmp([24, 22, 30])).map_err(|e| e.to_string())?;
                            println!("[miroir] image {}x{} : {}", cv.w, cv.h, file.display());
                        }
                        None => println!("[miroir] ChatFrame{} : rien à dessiner", u.frame),
                    }
                }
            }
            Ok(true)
        }
        "install" => {
            // Sans --wow : le ou les dossiers de WoW trouvés sur ce PC ; sans --flavor : toutes
            // les versions du jeu présentes.
            let roots: Vec<PathBuf> = match args.value("--wow") {
                Some(w) => vec![PathBuf::from(w)],
                None => {
                    let found = install::find_wow();
                    if found.is_empty() {
                        return Err("World of Warcraft introuvable sur ce PC : préciser --wow <dossier World of Warcraft>".into());
                    }
                    for f in &found {
                        println!("World of Warcraft trouvé : {}", f.display());
                    }
                    found
                }
            };
            let iface = match args.value("--interface") {
                Some(v) => Some(
                    v.parse::<u32>()
                        .map_err(|_| format!("interface invalide : {v}"))?,
                ),
                None => None,
            };
            for root in &roots {
                install::install(root, args.value("--flavor"), iface)?;
            }
            Ok(true)
        }
        "addon" => {
            // État de l'addon dans chaque version du jeu, sans rien installer.
            let roots: Vec<PathBuf> = match args.value("--wow") {
                Some(w) => vec![PathBuf::from(w)],
                None => install::find_wow(),
            };
            if roots.is_empty() {
                println!("World of Warcraft introuvable sur ce PC (préciser --wow <dossier>).");
            }
            for r in &roots {
                println!("{}", install::status_report(r));
            }
            Ok(true)
        }
        "fonts" => fonts::run(&fonts::Options {
            wow: PathBuf::from(
                args.value("--wow")
                    .ok_or("préciser --wow <dossier World of Warcraft>")?,
            ),
            flavor: args
                .value("--flavor")
                .ok_or("préciser --flavor (_forever_, _retail_, _classic_era_...)")?
                .to_string(),
            locale: args.value("--locale").map(String::from),
            files: args.values("--file"),
            fdids: args
                .values("--fdid")
                .iter()
                .map(|v| v.parse().map_err(|_| format!("FileDataID invalide : {v}")))
                .collect::<Result<_, _>>()?,
            out: args.value("--out").map(PathBuf::from),
        }),
        "materiel" | "hardware" => Ok(hw::run()),
        "estimation" | "plan" => plan::run(match (args.value("--wow"), args.value("--flavor")) {
            (Some(w), Some(f)) => Some(PathBuf::from(w).join(f)),
            (Some(w), None) => Some(PathBuf::from(w)),
            _ => None,
        }),
        "installation" => catalog::run(
            args.flag("--telecharger"),
            args.flag("--seulement-low"),
            args.value("--dossier").map(PathBuf::from),
            args.value("--modele"),
            args.value("--moteur"),
        ),
        "bench" => bench::run(&bench::Options {
            llama: args.value("--llama").map(PathBuf::from),
            runtime: args.value("--moteur").unwrap_or("vulkan").to_string(),
            model: PathBuf::from(args.value("--modele").ok_or("préciser --modele <fichier .gguf>")?),
            cpu: args.flag("--cpu"),
            court: args.flag("--court"),
            low: args.flag("--low"),
            fps: args.flag("--fps"),
            forcer: args.flag("--forcer"),
            port: args.num("--port", 8089)?,
            out: PathBuf::from(args.value("--sortie").unwrap_or("resultats-s4b")),
            tuning: args.tuning()?,
            pace: args.value("--rythme").map(|v| v.replace(',', ".").parse::<f64>().map_err(|_| format!("--rythme invalide : {v}"))).transpose()?,
            threads: args.value("--fils").map(|v| v.parse::<usize>().map_err(|_| format!("--fils invalide : {v}"))).transpose()?,
            regulate: !args.flag("--sans-regulateur"),
            bursts: args.bursts()?,
            status: None,
            stop: None,
            from_game: false,
        }),
        "interface" => ui::run(ui::Options {
            dir: args.value("--dossier").map(PathBuf::from).unwrap_or_else(catalog::models_dir),
            window: !args.flag("--sans-fenetre"),
            demo: args.flag("--demo"),
            port: args.num("--port", 0)?,
        }),
        "tests-en-jeu" => {
            #[cfg(windows)]
            {
                ingame::run(
                    args.value("--dossier").map(PathBuf::from).unwrap_or_else(catalog::models_dir),
                    args.value("--sortie").map(PathBuf::from).unwrap_or_else(|| fonts::cache_root().join("resultats-s4b")),
                )
            }
            #[cfg(not(windows))]
            {
                Err("tests en jeu : Windows seulement".into())
            }
        }
        "diagnostic" => diag::run(
            match (args.value("--wow"), args.value("--flavor")) {
                (Some(w), Some(f)) => Some(PathBuf::from(w).join(f).join("Errors")),
                _ => None,
            },
            args.num("--heures", 12)?,
        ),
        "essai" => {
            let f = args.value("--modele").ok_or("préciser --modele <fichier .gguf>")?;
            let file = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let verdict = args
                .value("--verdict")
                .and_then(catalog::Verdict::parse)
                .ok_or("préciser --verdict ok|lourd|plantage|memoire")?;
            let place = match args.value("--sur").unwrap_or("carte") {
                "carte" | "gpu" => catalog::Place::Gpu,
                "processeur" | "cpu" => catalog::Place::Cpu,
                o => return Err(format!("--sur carte|processeur, pas « {o} »")),
            };
            let t = catalog::Trial {
                verdict,
                when: catalog::now_secs(),
                drop: None,
                detail: args.value("--note").unwrap_or("noté à la main").to_string(),
            };
            catalog::record_trial(&file, place, &t);
            println!("Noté pour ce PC : {file} {} : {}", place.label(), t.summary());
            Ok(true)
        }
        "fixture-casc" => {
            // Harnais : fausse installation CASC pour la validation croisée du format.
            let out = PathBuf::from(args.value("--out").ok_or("préciser --out")?);
            casc::fixture::build(&out, &casc::fixture::default_entries())?;
            println!("installation synthétique : {}", out.display());
            Ok(true)
        }
        "decode-raw" => {
            let file = args.0.get(1).ok_or(USAGE)?;
            let w: usize = args.0.get(2).and_then(|v| v.parse().ok()).ok_or(USAGE)?;
            let h: usize = args.0.get(3).and_then(|v| v.parse().ok()).ok_or(USAGE)?;
            let data = std::fs::read(file).map_err(|e| format!("{file} : {e}"))?;
            if data.len() != w * h * 4 {
                return Err(format!(
                    "taille {} octets, attendu {}",
                    data.len(),
                    w * h * 4
                ));
            }
            match decode::decode(&decode::Image {
                w,
                h,
                stride: w * 4,
                bgra: &data,
            }) {
                Ok(d) => {
                    let f = &d.frame;
                    println!(
                        "type={} seq={} longueur={} bloc={} colonnes={}",
                        f.ftype,
                        f.seq,
                        f.payload.len(),
                        d.geometry.block,
                        d.geometry.cols
                    );
                    let mut live = live::Live::new(None);
                    live.feed(&decode::Image {
                        w,
                        h,
                        stride: w * 4,
                        bgra: &data,
                    });
                    if f.ftype == proto::FRAME_TEST {
                        let ok = f.payload
                            == proto::test_payload(f.seq, proto::capacity(d.geometry.cols).2);
                        println!(
                            "charge utile de test {}",
                            if ok { "conforme" } else { "FAUSSE" }
                        );
                        return Ok(ok);
                    }
                    Ok(true)
                }
                Err(e) => Err(format!("décodage impossible : {}", e.label())),
            }
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
