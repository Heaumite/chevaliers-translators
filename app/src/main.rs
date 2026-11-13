//! ChevaliersTranslators.exe : l'application du joueur.
//!
//! Sans console : une icône près de l'horloge (état, fenêtre des modèles, journal, Quitter). Au
//! démarrage, l'addon est posé ou mis à jour dans toutes les versions de WoW trouvées ; la
//! traduction démarre dès que WoW est ouvert, pour sa version (Retail, Forever, Classic...), et
//! s'arrête quand il est fermé (mémoire vidéo rendue). Ce qu'affichait la console de la phase 0
//! va dans le journal (`%LOCALAPPDATA%\ChevaliersTranslators\journal`).
//!
//! `ChevaliersTranslators.exe interface` : fenêtre des modèles (ouverte depuis le menu de l'icône,
//! et à chaque lancement par le joueur ; `--discret` : sans elle). Premier lancement : raccourci
//! sur le bureau.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("Chevaliers Translators : application Windows (outils de développement : ct-outils).");
    std::process::ExitCode::FAILURE
}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    app::main()
}

/// Journal de l'application : sorties standard redirigées vers un fichier, les plus récents gardés.
#[cfg(windows)]
mod journal {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    /// Journaux gardés pour chaque sorte (application, interface).
    const KEEP: usize = 10;

    pub fn dir() -> PathBuf {
        ct::fonts::cache_root().join("journal")
    }

    fn now() -> SYSTEMTIME {
        unsafe {
            let mut t: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut t);
            t
        }
    }

    /// « 10/11/2026 21:04:33 »
    pub fn stamp() -> String {
        let t = now();
        format!("{:02}/{:02}/{} {}", t.wDay, t.wMonth, t.wYear, clock())
    }

    /// « 21:04:33 »
    pub fn clock() -> String {
        let t = now();
        format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
    }

    /// Nouveau journal `<sorte>-AAAA-MM-JJ_HH-MM-SS.txt` ; les plus anciens au-delà de `KEEP` sont
    /// effacés. Tout ce que l'application écrit (println!, erreurs, plantage) y va.
    pub fn open(kind: &str) -> Option<PathBuf> {
        let dir = dir();
        std::fs::create_dir_all(&dir).ok()?;
        let prefix = format!("{kind}-");
        let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".txt")))
            .collect();
        old.sort();
        while old.len() >= KEEP {
            let _ = std::fs::remove_file(old.remove(0));
        }
        let t = now();
        let path = dir.join(format!(
            "{prefix}{}-{:02}-{:02}_{:02}-{:02}-{:02}.txt",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        ));
        let file = std::fs::File::create(&path).ok()?;
        // Le journal s'écrit par sa propre poignée (macro `say!`) ; la sortie d'erreur (plantage)
        // et la sortie standard y sont aussi redirigées, par une copie de la poignée.
        if let Ok(copy) = file.try_clone() {
            ct::set_journal(copy);
        }
        use std::os::windows::io::IntoRawHandle;
        let h = file.into_raw_handle();
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, h as _);
            SetStdHandle(STD_ERROR_HANDLE, h as _);
        }
        Some(path)
    }
}

#[cfg(windows)]
mod app {
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use ct::tray_win::{self, Tray, TITLE};
    use ct::{capture_win, catalog, engine, fonts, install, lang, live, mirror, ui};
    use ct::jeux_win as live_games;

    use crate::journal;

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// WoW fermé depuis ce temps : traduction arrêtée, mémoire vidéo rendue. Un jeu relancé aussitôt
    /// (après la pose de l'addon, par exemple) garde le modèle chargé.
    const GONE: Duration = Duration::from_secs(20);

    pub fn main() -> ExitCode {
        let interface = std::env::args().nth(1).is_some_and(|a| a == "interface");
        let log = journal::open(if interface { "interface" } else { "application" });
        panic_hook(log.clone());
        ct::say!("{TITLE} {VERSION} - {}", journal::stamp());
        if interface {
            // Fenêtre des modèles : processus à part, son moteur web voulant le fil principal.
            return match ui::run(ui::Options { dir: catalog::models_dir(), window: true, demo: false, port: 0 }) {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    ct::say!("{e}");
                    tray_win::message(&format!("Fenêtre des modèles : {e}"), true);
                    ExitCode::FAILURE
                }
            };
        }
        // Avant toute fenêtre : coordonnées en pixels physiques (capture du bandeau, chat miroir).
        unsafe {
            use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let after_update = std::env::args().any(|a| a == "--apres-maj");
        let mut instance = single_instance();
        // Lancée par une mise à jour (0.16) : l'ancienne version est en train de s'arrêter.
        if instance.is_none() && after_update {
            for _ in 0..60 {
                std::thread::sleep(Duration::from_millis(250));
                instance = single_instance();
                if instance.is_some() {
                    break;
                }
            }
        }
        // Une autre version tourne déjà (mise à jour lancée sans quitter l'ancienne, 0.15.1) : elle
        // s'arrête, et celle-ci prend la suite (sinon l'ancienne continuait, addon compris).
        if instance.is_none() {
            if let Some(v) = tray_win::running_version().filter(|v| v != VERSION) {
                ct::say!("version {v} déjà lancée : arrêt demandé, la {VERSION} prend la suite");
                let wait_free = |tries: usize| -> Option<usize> {
                    for _ in 0..tries {
                        std::thread::sleep(Duration::from_millis(250));
                        if let Some(h) = single_instance() {
                            return Some(h);
                        }
                    }
                    None
                };
                if tray_win::ask_running_to_make_way() {
                    instance = wait_free(40);
                }
                // Version d'avant la 0.15.1 (demande d'arrêt inconnue) : arrêtée de force.
                if instance.is_none() && tray_win::terminate_running() {
                    ct::say!("ancienne version arrêtée de force");
                    instance = wait_free(20);
                }
            }
        }
        let Some(_instance) = instance else {
            // Second lancement (raccourci du bureau, double-clic) : l'application déjà lancée
            // ouvre sa fenêtre des modèles (0.15).
            if tray_win::ask_running_to_open_models() {
                ct::say!("déjà lancée : fenêtre des modèles demandée à l'application lancée");
                return ExitCode::SUCCESS;
            }
            ct::say!("déjà lancée : arrêt");
            tray_win::message(
                "Chevaliers Translators est déjà lancé : son icône est près de l'horloge \
                 (flèche ^ si Windows l'a rangée).",
                false,
            );
            return ExitCode::SUCCESS;
        };
        if let Some(p) = log {
            Tray::set_journal(p);
        }
        let tray = match Tray::new("En attente de World of Warcraft") {
            Ok(t) => t,
            Err(e) => {
                ct::say!("{e}");
                tray_win::message(&format!("Icône près de l'horloge impossible : {e}"), true);
                return ExitCode::FAILURE;
            }
        };
        desktop_shortcut();
        // Ouverture par le joueur (0.15) : la fenêtre des modèles s'affiche ; « --discret » pour
        // un lancement sans fenêtre.
        if !std::env::args().any(|a| a == "--discret") {
            Tray::open_models();
        }
        // Mises à jour depuis GitHub (0.16) : l'ancien exécutable laissé par la précédente est
        // retiré, puis recherche en fond.
        if let Ok(exe) = std::env::current_exe() {
            ct::maj::cleanup_old(&exe);
            if after_update {
                let from = std::env::args().skip_while(|a| a != "--apres-maj").nth(1).unwrap_or_default();
                ct::say!("[mise à jour] installée : {from} -> {VERSION}");
                tray.notify("Chevaliers Translators mis à jour", &format!("Version {VERSION} installée depuis GitHub ; l'addon suit dans toutes les versions de WoW."), false);
            }
            if ct::maj::is_dev_build(&exe) {
                ct::say!("[mise à jour] version de développement ({}) : pas d'installation automatique", exe.display());
            } else if !ct::maj::auto_enabled() {
                ct::say!("[mise à jour] automatiques coupées (menu de l'icône) ; « Rechercher une mise à jour » reste possible");
            }
            ct::maj::start(VERSION, exe);
        }
        run(&tray);
        ct::say!("{} fin", journal::clock());
        ExitCode::SUCCESS
    }

    /// Raccourci « Chevaliers Translators » sur le bureau (0.15) : posé au premier lancement.
    fn desktop_shortcut() {
        let Ok(exe) = std::env::current_exe() else { return };
        match ct::raccourci::ensure(&exe) {
            Ok(Some(link)) => ct::say!("Raccourci posé sur le bureau : {}", link.display()),
            Ok(None) => {}
            Err(e) => ct::say!("Raccourci du bureau impossible : {e}"),
        }
    }

    /// Boucle de l'application : addon posé, puis traduction tant que WoW est ouvert.
    fn run(tray: &Tray) {
        let mut roots = install_everywhere(tray);
        log_screens();
        start_downloads(tray);
        let mut last_error: Option<String> = None;
        let mut said: Option<Instant> = None;
        let mut downloading = ct::telechargements::status().active;
        let mut shown_at = Instant::now();
        loop {
            Tray::pump();
            if Tray::quit_asked() {
                break;
            }
            downloads_tick(tray, &mut downloading, &mut shown_at, "En attente de World of Warcraft");
            // Présence écrite aussi WoW fermé : au lancement du jeu, l'addon la trouve déjà
            // récente et la traduction démarre à la connexion, sans /reload.
            if said.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
                for r in &roots {
                    install::presence(r, true);
                }
                said = Some(Instant::now());
            }
            update_events(tray);
            if !capture_win::wow_windows().is_empty() {
                translate_all(tray, &mut roots, &mut last_error);
                if Tray::quit_asked() {
                    break;
                }
                tray.set_status("En attente de World of Warcraft");
            } else if install_update(tray) {
                // La nouvelle version est lancée : celle-ci s'arrête.
                break;
            }
            wait(Duration::from_millis(500));
        }
        // Application quittée : présence remise à zéro, le prochain /reload rend le chat officiel.
        for r in &roots {
            install::presence(r, false);
        }
        if let Some(data) = Tray::uninstall_asked() {
            uninstall(&roots, data);
        }
    }

    /// Nouvelles du fil des mises à jour : bulles de l'icône.
    fn update_events(tray: &Tray) {
        for e in ct::maj::take_events() {
            match e {
                ct::maj::Event::UpToDate(v) => tray.notify("Pas de mise à jour", &format!("La version {v} est la dernière publiée sur GitHub."), false),
                ct::maj::Event::Found(v) => tray.notify("Mise à jour trouvée", &format!("Version {v} publiée sur GitHub : téléchargement en cours."), false),
                ct::maj::Event::Ready(v) => {
                    let when = if capture_win::wow_windows().is_empty() { "elle s'installe maintenant" } else { "elle s'installera à la fermeture de WoW" };
                    tray.notify("Mise à jour prête", &format!("Version {v} téléchargée et vérifiée : {when}."), false);
                }
                ct::maj::Event::Problem(p) => tray.notify("Mise à jour impossible", &p, true),
            }
        }
    }

    /// Mise à jour prête et aucun WoW ouvert : le nouvel exécutable prend la place de celui-ci et
    /// est lancé. Vrai si celui-ci doit s'arrêter.
    fn install_update(tray: &Tray) -> bool {
        let Some((v, new_exe)) = ct::maj::ready() else { return false };
        let Ok(exe) = std::env::current_exe() else { return false };
        if ct::maj::is_dev_build(&exe) {
            return false;
        }
        ct::say!("{} [mise à jour] installation de la {v} : {}", journal::clock(), exe.display());
        if let Err(e) = ct::maj::apply(&new_exe, &exe) {
            ct::say!("[mise à jour] {e}");
            tray.notify("Mise à jour impossible", &format!("Version {v} : {e}. Elle est sur GitHub : {}", ct::maj::REPO), true);
            // Plus d'autre essai pendant cette session.
            ct::maj::forget_ready();
            return false;
        }
        let mut c = std::process::Command::new(&exe);
        c.args(["--discret", "--apres-maj", VERSION]);
        match c.spawn() {
            Ok(_) => {
                ct::say!("[mise à jour] {v} lancée : arrêt de la {VERSION}");
                Tray::quit_now();
                true
            }
            Err(e) => {
                ct::say!("[mise à jour] lancement de la {v} impossible : {e}");
                tray.notify("Mise à jour installée", &format!("Version {v} en place : relance Chevaliers Translators (raccourci du bureau)."), true);
                ct::maj::forget_ready();
                false
            }
        }
    }

    /// Désinstallation (0.14.3, menu de l'icône) : l'addon et sa présence retirés de toutes les
    /// versions de WoW ; avec `data`, les réglages de l'addon et les données de l'application.
    /// Ce qui est en cours d'usage (l'exécutable lancé depuis le dossier de l'addon, le journal
    /// ouvert) est retiré quelques secondes après l'arrêt, par une commande Windows détachée.
    fn uninstall(roots: &[PathBuf], data: bool) {
        let targets = install::uninstall_targets(roots, data, &ct::fonts::cache_root());
        ct::say!("{} désinstallation{} : {} élément(s)", journal::clock(), if data { " complète" } else { " de l'addon" }, targets.len());
        for t in &targets {
            ct::say!("  {}", t.display());
        }
        if let Some(link) = ct::raccourci::remove() {
            ct::say!("  {} (raccourci du bureau)", link.display());
        }
        let later = install::remove_now(&targets);
        if !later.is_empty() {
            let mut cmds = vec!["ping -n 4 127.0.0.1 >nul".to_string()];
            for p in &later {
                let p = p.display().to_string();
                cmds.push(if std::path::Path::new(&p).is_dir() { format!("rmdir /s /q \"{p}\"") } else { format!("del /f /q \"{p}\"") });
            }
            use std::os::windows::process::CommandExt;
            let mut c = std::process::Command::new("cmd.exe");
            c.raw_arg(format!("/d /s /c \"{}\"", cmds.join(" & ")));
            ct::quiet(&mut c);
            if let Err(e) = c.spawn() {
                ct::say!("  retrait après l'arrêt impossible : {e}");
            }
        }
        let exe = std::env::current_exe().ok();
        let inside = exe.as_ref().is_some_and(|e| targets.iter().any(|t| e.starts_with(t)));
        let mut text = String::from("Chevaliers Translators est désinstallé : l'addon est retiré de toutes les versions de WoW.");
        if data {
            text += "\nModèles, moteur, journaux et réglages de l'addon supprimés.";
        }
        if let (Some(e), false) = (exe, inside) {
            text += &format!("\n\nTu peux supprimer l'application elle-même :\n{}", e.display());
        }
        tray_win::message(&text, false);
    }

    /// Attente qui laisse vivre l'icône.
    fn wait(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end && !Tray::quit_asked() {
            Tray::pump();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Traduction de toutes les fenêtres de WoW ouvertes (0.15 : plusieurs jeux à la fois, chacun
    /// dans sa version et la langue de son client, un seul modèle pour tous), jusqu'à Quitter ou
    /// la fermeture du dernier jeu.
    fn translate_all(tray: &Tray, roots: &mut Vec<PathBuf>, last_error: &mut Option<String>) {
        use std::collections::HashSet;
        let Some(first) = capture_win::wow_windows().into_iter().find_map(capture_win::game_of) else { return };
        if let Some(missing) = missing_parts().filter(|_| !ct::telechargements::status().active) {
            tray.notify(
                "Traduction impossible pour l'instant",
                &format!("{missing}. Clic sur l'icône, puis « Modèles de traduction... ». Le chat reste tel quel en attendant."),
                true,
            );
        }
        let cfg = engine::Config {
            wow: Some(first.0.join(&first.1)),
            models: catalog::models_dir(),
            port: free_port(),
            log: engine::log_path(),
            phrases: Some(engine::phrases_path()),
        };
        let mut gone: Option<Instant> = None;
        let mut why = "";
        let mut downloading = ct::telechargements::status().active;
        let mut shown_at = Instant::now();
        let status = std::cell::RefCell::new(String::from("En attente de World of Warcraft"));
        let mut ensured: HashSet<(String, String)> = HashSet::new();
        let mut seen: HashSet<isize> = HashSet::new();
        let mut found_roots: Vec<PathBuf> = Vec::new();
        let mut last_count = usize::MAX;
        let mut games = || -> Vec<live_games::GameSpec> {
            let mut out = Vec::new();
            for h in capture_win::wow_windows() {
                let Some((root, flavor)) = capture_win::game_of(h) else { continue };
                let key = (root.to_string_lossy().to_lowercase(), flavor.clone());
                if ensured.insert(key) {
                    if !found_roots.iter().any(|r| same_path(r, &root)) {
                        found_roots.push(root.clone());
                    }
                    ensure_addon(tray, &root, &flavor);
                }
                let locale = fonts::client_locale_of(&root, &flavor);
                let client = lang::from_locale(&locale).unwrap_or("en");
                let label = flavor_label(&flavor);
                if seen.insert(h as isize) {
                    ct::say!(
                        "{} WoW {label} ouvert ({}, client {locale}) : traduction vers {}",
                        journal::clock(),
                        root.display(),
                        client.to_uppercase()
                    );
                    // Écran de WoW (0.14) : reconnu dès l'ouverture du jeu ; sur une télévision,
                    // l'addon applique le dispositif télévision.
                    screen_of_wow(h);
                    log_display_mode(&root, &flavor);
                }
                out.push(live_games::GameSpec {
                    hwnd: h as isize,
                    addons: Some(install::addons_dir(&root, &flavor)),
                    fonts: live::FontSource::Game { wow: root.clone(), flavor: flavor.clone() },
                    client,
                    label,
                });
            }
            if out.len() != last_count {
                last_count = out.len();
                *status.borrow_mut() = match out.as_slice() {
                    [] => "En attente de World of Warcraft".to_string(),
                    [g] => format!("WoW {} : traduction vers {}", g.label, g.client.to_uppercase()),
                    many => format!("{} jeux traduits : {}", many.len(), many.iter().map(|g| g.label.as_str()).collect::<Vec<_>>().join(", ")),
                };
                tray.set_status(&status.borrow());
                if out.len() > 1 {
                    ct::say!("{} {} fenêtres de WoW traduites en même temps (un seul modèle pour toutes)", journal::clock(), out.len());
                }
            }
            out
        };
        let mut checked = Instant::now();
        let r = live_games::run_games(
            0,
            48,
            // Capture rapide toutes les 8 ms quand le bandeau change, 40 ms au repos (0.15).
            8,
            false,
            // Calage du 08/11/2026 (Forever, Arial Narrow 16 px) : texte du miroir 1 px à gauche.
            mirror::RenderOpts { dx: 1.0, dy: 0.0, plate: None },
            Some((cfg, true)),
            &mut games,
            &mut || {
                if Tray::quit_asked() {
                    why = "Quitter";
                    return false;
                }
                downloads_tick(tray, &mut downloading, &mut shown_at, &status.borrow());
                update_events(tray);
                let now = Instant::now();
                if now.duration_since(checked) < Duration::from_secs(1) {
                    return true;
                }
                checked = now;
                if capture_win::wow_windows().is_empty() {
                    let since = *gone.get_or_insert(now);
                    if now.duration_since(since) >= GONE {
                        why = "WoW fermé";
                        return false;
                    }
                } else {
                    gone = None;
                }
                true
            },
            false,
        );
        for r in found_roots {
            if !roots.iter().any(|x| same_path(x, &r)) {
                roots.push(r);
            }
        }
        match r {
            Ok(()) => {
                ct::say!("{} traduction arrêtée : {why}", journal::clock());
                *last_error = None;
            }
            Err(e) => {
                ct::say!("{} traduction interrompue : {e} ; nouvel essai dans 5 s", journal::clock());
                // Une bulle par erreur nouvelle, pas à chaque essai.
                if last_error.as_deref() != Some(e.as_str()) {
                    tray.notify("Traduction interrompue", &format!("{e}. Nouvel essai dans 5 s."), true);
                    *last_error = Some(e);
                }
                wait(Duration::from_secs(5));
            }
        }
    }

    /// Addon posé ou mis à jour dans toutes les versions des dossiers de WoW trouvés, rendus.
    fn install_everywhere(tray: &Tray) -> Vec<PathBuf> {
        let mut roots = install::find_wow();
        // Application lancée depuis le dossier de l'addon (elle est livrée dans son zip) : ce
        // dossier de WoW, même installé ailleurs que d'habitude.
        if let Some(r) = wow_of_exe() {
            if !roots.iter().any(|x| same_path(x, &r)) {
                roots.push(r);
            }
        }
        if roots.is_empty() {
            ct::say!("World of Warcraft introuvable dans les dossiers habituels : l'addon sera posé à l'ouverture du jeu.");
            return roots;
        }
        // Jeux ouverts (plusieurs possibles, 0.15) : une version du jeu ouverte qui reçoit une
        // mise à jour est annoncée sur l'écran.
        let open: Vec<(PathBuf, String)> = capture_win::wow_windows().into_iter().filter_map(capture_win::game_of).collect();
        for root in &roots {
            ct::say!("World of Warcraft : {}", root.display());
            // 0.15.1 : comparaison fichier par fichier, pas seulement du numéro de version.
            let before: Vec<(String, install::Check)> = install::flavor_dirs(root).into_iter().map(|f| {
                let c = install::check(root, &f);
                (f, c)
            }).collect();
            if before.iter().all(|(_, c)| c.up_to_date()) {
                ct::say!("  addon {VERSION} déjà en place, fichier par fichier, dans toutes les versions");
                continue;
            }
            for (f, c) in before.iter().filter(|(_, c)| !c.up_to_date()) {
                ct::say!("  {f} : {} -> à remplacer", c.describe());
            }
            if let Err(e) = install::install(root, None, None) {
                ct::say!("  installation : {e}");
                tray.notify("Addon non mis à jour", &format!("{} : {e}", root.display()), true);
            }
            // Vérification : chaque version du jeu doit porter exactement les fichiers de cette
            // version de l'addon, pour qu'une ancienne ne se charge pas à sa place.
            for (f, was) in &before {
                let now = install::check(root, f);
                if now.up_to_date() {
                    if !was.up_to_date() {
                        ct::say!("  {f} : {} -> {VERSION}, vérifié", was.describe());
                    }
                } else {
                    ct::say!("  {f} : ATTENTION, encore {} après la mise à jour", now.describe());
                    tray.notify(
                        "Addon à vérifier",
                        &format!("WoW {} : l'addon n'a pas pu être remplacé ({}). Ferme le jeu, puis relance l'application.", flavor_label(f), now.describe()),
                        true,
                    );
                }
            }
            // Copies de l'application rangées dans les dossiers de l'addon (paquet décompressé dans
            // une version du jeu) : remplacées par celle-ci (0.16, toutes les installations suivent).
            refresh_exe_copies(root);
            // Jeu déjà ouvert dans une version qui vient d'être mise à jour : un seul message, une
            // fois la mise à jour vérifiée. /reload suffit si la liste des fichiers n'a pas changé ;
            // sinon il faut redémarrer le jeu (il ne relit la liste qu'à son démarrage).
            for (r, f) in &open {
                if !same_path(r, root) {
                    continue;
                }
                let Some((_, was)) = before.iter().find(|(x, _)| x == f) else { continue };
                if !was.up_to_date() && install::check(root, f).up_to_date() {
                    announce(tray, f, was.version.is_some(), was.files_changed);
                }
            }
        }
        roots
    }

    /// Copies de l'exécutable dans les dossiers de l'addon de chaque version du jeu : mises à jour.
    fn refresh_exe_copies(root: &Path) {
        let Ok(exe) = std::env::current_exe() else { return };
        if ct::maj::is_dev_build(&exe) {
            return;
        }
        let Ok(me) = std::fs::read(&exe) else { return };
        for f in install::flavor_dirs(root) {
            let copy = install::addons_dir(root, &f).join(install::ADDON).join("ChevaliersTranslators.exe");
            if !copy.is_file() || same_path(&copy, &exe) || std::fs::read(&copy).ok().as_deref() == Some(me.as_slice()) {
                continue;
            }
            match std::fs::write(&copy, &me) {
                Ok(()) => ct::say!("  {f} : application mise à jour en {VERSION} ({})", copy.display()),
                Err(e) => ct::say!("  {f} : copie de l'application non remplacée ({}) : {e}", copy.display()),
            }
        }
    }

    /// Jeu ouvert : addon présent et à jour dans sa version (dossier de WoW inhabituel, version du
    /// jeu ajoutée depuis le démarrage), vérifié fichier par fichier.
    fn ensure_addon(tray: &Tray, root: &Path, flavor: &str) {
        let before = install::check(root, flavor);
        if before.up_to_date() {
            return;
        }
        ct::say!("  {flavor} : {} -> à remplacer", before.describe());
        match install::install(root, Some(flavor), None) {
            Ok(()) => {
                // Vérification : la nouvelle version doit bien être en place avant d'inviter au /reload.
                if install::check(root, flavor).up_to_date() {
                    announce(tray, flavor, before.version.is_some(), before.files_changed);
                } else {
                    tray.notify(
                        "Addon à vérifier",
                        &format!("WoW {} : l'addon n'a pas pu être mis à jour. Ferme le jeu, puis relance l'application.", flavor_label(flavor)),
                        true,
                    );
                }
            }
            Err(e) => tray.notify(
                "Addon non installé",
                &format!("WoW {} : {e}", flavor_label(flavor)),
                true,
            ),
        }
    }

    /// `files_changed` : de nouveaux fichiers d'addon (ou une liste changée) ; WoW ne les charge
    /// qu'à son redémarrage, /reload ne suffit pas (0.15.1).
    fn announce(tray: &Tray, flavor: &str, update: bool, files_changed: bool) {
        let label = flavor_label(flavor);
        if update && !files_changed {
            // Fichiers Lua relus à chaque connexion : /reload seulement si un personnage est
            // déjà connecté. Sans /reload, l'ancienne version de l'addon reste chargée.
            tray.notify("Addon mis à jour", &format!("WoW {label} : addon en version {VERSION}. Si tu es déjà en jeu, tape /reload pour charger la nouvelle version ; sinon rien à faire."), false);
        } else if update {
            tray.notify(
                "Addon mis à jour : redémarre WoW",
                &format!("WoW {label} : addon en version {VERSION}, avec de nouveaux fichiers. Quitte complètement le jeu et relance-le : /reload ne charge pas les nouveaux fichiers."),
                true,
            );
        } else {
            tray.notify("Addon installé", &format!("WoW {label} : redémarre le jeu pour charger l'addon Chevaliers Translators."), false);
        }
    }

    /// `<WoW>\_retail_\Interface\AddOns\ChevaliersTranslators\ChevaliersTranslators.exe` -> `<WoW>`.
    fn wow_of_exe() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let addons = exe.parent()?.parent()?;
        let root = addons.parent()?.parent()?.parent()?;
        let is = |p: &Path, name: &str| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name));
        (is(addons, "AddOns") && is(addons.parent()?, "Interface") && install::is_wow_root(root)).then(|| root.to_path_buf())
    }

    /// Premier lancement et suivants (0.14) : moteur et tous les modèles qui peuvent tourner sur ce
    /// PC, téléchargés en fond (bridés pendant le jeu), le pack low d'abord ; à la fin de chacun,
    /// le meilleur modèle installé devient actif (hors combat).
    fn start_downloads(tray: &Tray) {
        ct::telechargements::set_wow_probe(|| capture_win::wow_game().is_some());
        let hw = ct::hw::detect();
        let plan = ct::plan::for_this_pc(&hw, None);
        let dir = catalog::models_dir();
        let runtime = catalog::offer(&hw, &dir).runtime;
        let bytes = ct::telechargements::want(dir, Some(runtime), &plan.downloads);
        let names: Vec<&str> = plan.downloads.iter().map(|m| m.name).collect();
        ct::say!("{} modèles prévus pour ce PC : {} ; à télécharger : {}", journal::clock(), names.join(", "), catalog::go(bytes));
        if bytes > 0 {
            tray.notify(
                "Téléchargement des modèles de traduction",
                &format!(
                    "{} à télécharger ({}). La traduction démarre dès que le pack low est prêt ; le meilleur modèle pour ton PC sera activé à la fin.",
                    catalog::go(bytes),
                    names.join(", ")
                ),
                false,
            );
        }
    }

    /// Progression des téléchargements dans l'infobulle (toutes les 3 s) ; bulle à la fin.
    fn downloads_tick(tray: &Tray, downloading: &mut bool, shown_at: &mut Instant, idle: &str) {
        if shown_at.elapsed() < Duration::from_secs(3) {
            return;
        }
        *shown_at = Instant::now();
        let st = ct::telechargements::status();
        if st.active {
            tray.set_status(&format!("{idle}\nTéléchargement : {}", st.line()));
        } else if *downloading {
            tray.set_status(idle);
            match &st.problem {
                Some(p) => tray.notify("Téléchargement interrompu", &format!("{p}. Il reprendra au prochain lancement."), true),
                None => tray.notify("Modèles de traduction prêts", "Le meilleur modèle pour ton PC est actif (bascule hors combat si tu joues).", false),
            }
        }
        *downloading = st.active;
    }

    /// Écrans branchés, au journal dès le lancement : une télévision est connue avant que WoW
    /// s'ouvre.
    fn log_screens() {
        let all = capture_win::screens();
        ct::say!("{} [écran] {} écran(s) branché(s)", journal::clock(), all.len());
        for s in &all {
            ct::say!("[écran]   {}", s.describe());
        }
        if all.iter().any(|s| s.is_tv()) {
            ct::say!("[écran] télévision branchée : dispositif télévision si WoW s'ouvre dessus (le chat traduit recouvre celui du jeu)");
        }
    }

    /// Écran de WoW : dispositif télévision noté dans la présence (lue par l'addon à la connexion).
    fn screen_of_wow(h: windows_sys::Win32::Foundation::HWND) {
        if let Some(s) = capture_win::screen_of(h) {
            let tv = s.is_tv();
            install::set_screen_tv(tv);
            ct::say!(
                "{} [écran] WoW sur {}{}",
                journal::clock(),
                s.describe(),
                if tv { " : dispositif télévision" } else { "" }
            );
        }
    }

    /// Mode d'affichage de WoW (`gxWindow`, `gxMaximize`, `gxApi` dans Config.wtf), au journal.
    fn log_display_mode(root: &Path, flavor: &str) {
        let Ok(text) = std::fs::read_to_string(root.join(flavor).join("WTF").join("Config.wtf")) else { return };
        let cvar = |name: &str| {
            text.lines().find_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("SET ").or_else(|| l.strip_prefix("set "))?;
                let (k, v) = rest.split_once(' ')?;
                k.eq_ignore_ascii_case(name).then(|| v.trim().trim_matches('"').to_string())
            })
        };
        let mode = match (cvar("gxWindow").as_deref(), cvar("gxMaximize").as_deref()) {
            (Some("0"), _) => "plein écran",
            (_, Some("1")) => "fenêtré (plein écran)",
            (Some(_), _) => "fenêtré",
            _ => "inconnu",
        };
        ct::say!("{} [écran] WoW en mode {mode}, rendu {}", journal::clock(), cvar("gxApi").unwrap_or_else(|| "par défaut".into()));
    }

    /// Moteur llama.cpp ou modèle absents (téléchargés depuis la fenêtre des modèles).
    fn missing_parts() -> Option<&'static str> {
        let rdir = catalog::runtime_dir();
        if !["vulkan", "cpu"].iter().any(|r| catalog::runtime_present(&rdir, r)) {
            return Some("Moteur de traduction (llama.cpp) absent");
        }
        let gguf = std::fs::read_dir(catalog::models_dir())
            .map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("gguf"))))
            .unwrap_or(false);
        (!gguf).then_some("Aucun modèle de traduction installé")
    }

    /// « _retail_ » -> « Retail », « _classic_era_ » -> « Classic era ».
    fn flavor_label(flavor: &str) -> String {
        let s = flavor.trim_matches('_').replace('_', " ");
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().chain(c).collect(),
            None => flavor.to_string(),
        }
    }

    fn same_path(a: &Path, b: &Path) -> bool {
        let n = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        n(a) == n(b)
    }

    /// Port du serveur de traduction : le premier libre à partir de 8090 (celui d'une traduction
    /// précédente peut être encore en train de s'arrêter).
    fn free_port() -> u16 {
        (8090..8100).find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()).unwrap_or(8090)
    }

    /// Une seule application à la fois (deux captures et deux chats miroirs se gêneraient).
    fn single_instance() -> Option<usize> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\ChevaliersTranslators.Application".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
            if !h.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(h);
                return None;
            }
            // Gardée ouverte jusqu'à la fin de l'application.
            Some(h as usize)
        }
    }

    /// Plantage : message dans le journal (sortie d'erreur redirigée) et, sur le fil principal,
    /// une boîte de message (sans console, il passerait inaperçu).
    fn panic_hook(log: Option<PathBuf>) {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            default(info);
            if std::thread::current().name() == Some("main") {
                let where_ = log.as_ref().map(|p| format!("\n\nJournal : {}", p.display())).unwrap_or_default();
                tray_win::message(&format!("{TITLE} s'est arrêté sur une erreur :\n{info}{where_}"), true);
            }
        }));
    }
}
