//! ChevaliersTranslators.exe : l'application du joueur.
//!
//! Sans console : une icône près de l'horloge (état, fenêtre des modèles, journal, Quitter). Au
//! démarrage, l'addon est posé ou mis à jour dans toutes les versions de WoW trouvées ; la
//! traduction démarre dès que WoW est ouvert, pour sa version (Retail, Forever, Classic...), et
//! s'arrête quand il est fermé (mémoire vidéo rendue). Ce qu'affichait la console de la phase 0
//! va dans le journal (`%LOCALAPPDATA%\ChevaliersTranslators\journal`).
//!
//! `ChevaliersTranslators.exe interface` : fenêtre des modèles (ouverte depuis le menu de l'icône).

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
        let Some(_instance) = single_instance() else {
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
        run(&tray);
        ct::say!("{} fin", journal::clock());
        ExitCode::SUCCESS
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
            if let Some((root, flavor)) = capture_win::wow_game() {
                if !roots.iter().any(|r| same_path(r, &root)) {
                    roots.push(root.clone());
                }
                ensure_addon(tray, &root, &flavor);
                translate(tray, &root, &flavor, &mut last_error);
                if Tray::quit_asked() {
                    break;
                }
                tray.set_status("En attente de World of Warcraft");
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

    /// Traduction pour le jeu ouvert, jusqu'à Quitter, sa fermeture ou son changement de version.
    fn translate(tray: &Tray, root: &Path, flavor: &str, last_error: &mut Option<String>) {
        let locale = fonts::client_locale_of(root, flavor);
        let client = lang::from_locale(&locale).unwrap_or("en");
        let label = flavor_label(flavor);
        ct::say!(
            "{} WoW {label} ouvert ({}, client {locale}) : traduction vers {}",
            journal::clock(),
            root.display(),
            client.to_uppercase()
        );
        tray.set_status(&format!("WoW {label} : traduction vers {}", client.to_uppercase()));
        // Écran de WoW (0.14) : reconnu dès l'ouverture du jeu, avant la connexion du personnage.
        // Sur une télévision, l'addon applique le dispositif télévision (le chat traduit recouvre
        // celui du jeu). Plein écran ou fenêtré : rien à changer dans WoW.
        screen_of_wow();
        log_display_mode(root, flavor);
        if let Some(missing) = missing_parts().filter(|_| !ct::telechargements::status().active) {
            tray.notify(
                "Traduction impossible pour l'instant",
                &format!("{missing}. Clic sur l'icône, puis « Modèles de traduction... ». Le chat reste tel quel en attendant."),
                true,
            );
        }
        let cfg = engine::Config {
            wow: Some(root.join(flavor)),
            models: catalog::models_dir(),
            port: free_port(),
            log: engine::log_path(),
            phrases: Some(engine::phrases_path()),
        };
        let mut gone: Option<Instant> = None;
        let mut checked = Instant::now();
        let mut why = "";
        let mut downloading = ct::telechargements::status().active;
        let mut shown_at = Instant::now();
        let status = format!("WoW {label} : traduction vers {}", client.to_uppercase());
        let r = live::run(
            0,
            48,
            4,
            Some(install::addons_dir(root, flavor)),
            live::FontSource::Game { wow: root.to_path_buf(), flavor: flavor.to_string() },
            false,
            // Calage du 08/11/2026 (Forever, Arial Narrow 16 px) : texte du miroir 1 px à gauche.
            mirror::RenderOpts { dx: 1.0, dy: 0.0, plate: None },
            Some((cfg, client, true)),
            &mut || {
                if Tray::quit_asked() {
                    why = "Quitter";
                    return false;
                }
                downloads_tick(tray, &mut downloading, &mut shown_at, &status);
                let now = Instant::now();
                if now.duration_since(checked) < Duration::from_secs(1) {
                    return true;
                }
                checked = now;
                match capture_win::wow_game() {
                    Some((r, f)) if same_path(&r, root) && f == flavor => {
                        gone = None;
                        true
                    }
                    Some(_) => {
                        why = "WoW ouvert dans une autre version";
                        false
                    }
                    None => {
                        let since = *gone.get_or_insert(now);
                        if now.duration_since(since) >= GONE {
                            why = "WoW fermé";
                            false
                        } else {
                            true
                        }
                    }
                }
            },
        );
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
        let open = capture_win::wow_game();
        for root in &roots {
            ct::say!("World of Warcraft : {}", root.display());
            let before = install::status(root);
            if before.iter().all(|(_, v)| v.as_deref() == Some(VERSION)) {
                ct::say!("  addon {VERSION} déjà en place dans toutes les versions");
                continue;
            }
            if let Err(e) = install::install(root, None, None) {
                ct::say!("  installation : {e}");
                continue;
            }
            // Jeu déjà ouvert : dire au joueur comment charger l'addon.
            if let Some((r, f)) = &open {
                if same_path(r, root) {
                    if let Some((_, v)) = before.iter().find(|(x, _)| x == f) {
                        if v.as_deref() != Some(VERSION) {
                            announce(tray, f, v.is_some());
                        }
                    }
                }
            }
        }
        roots
    }

    /// Jeu ouvert : addon présent et à jour dans sa version (dossier de WoW inhabituel, version du
    /// jeu ajoutée depuis le démarrage).
    fn ensure_addon(tray: &Tray, root: &Path, flavor: &str) {
        let before = install::addon_version(root, flavor);
        if before.as_deref() == Some(VERSION) {
            return;
        }
        match install::install(root, Some(flavor), None) {
            Ok(()) => announce(tray, flavor, before.is_some()),
            Err(e) => tray.notify(
                "Addon non installé",
                &format!("WoW {} : {e}", flavor_label(flavor)),
                true,
            ),
        }
    }

    fn announce(tray: &Tray, flavor: &str, update: bool) {
        let label = flavor_label(flavor);
        if update {
            // Fichiers Lua relus à chaque connexion : /reload seulement si un personnage est
            // déjà connecté.
            tray.notify("Addon mis à jour", &format!("WoW {label} : si tu es déjà connecté à un personnage, tape /reload ; sinon rien à faire."), false);
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
    fn screen_of_wow() {
        if let Some(s) = capture_win::wow_screen() {
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
