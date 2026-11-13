//! Chevaliers Translators : traduction du chat de World of Warcraft sur le PC du joueur.
//!
//! Bibliothèque commune à l'application (`ChevaliersTranslators.exe`) et aux outils de
//! développement (`ct-outils.exe`) : bandeau de pixels de l'addon, chat miroir, détection des
//! langues, traduction locale (llama.cpp) et choix des modèles.

pub mod bandeau;
pub mod bench;
pub mod brand;
pub mod res_icone;
#[cfg(test)]
mod res_icone_tests;
pub mod burst;
pub mod casc;
pub mod catalog;
pub mod decode;
pub mod diag;
pub mod ecran;
pub mod engine;
pub mod fonts;
pub mod fps;
pub mod glossary;
pub mod gouverneur;
pub mod hub;
pub mod hw;
pub mod icons;
pub mod ingame;
pub mod install;
pub mod lang;
pub mod live;
pub mod llama;
pub mod md5;
pub mod mirror;
pub mod panel;
pub mod plan;
pub mod mesures;
pub mod conseil;
pub mod telechargements;
pub mod proto;
pub mod raccourci;
pub mod regul;
pub mod render;
pub mod selftest;
pub mod ttf;
pub mod ui;
#[cfg(windows)]
pub mod capture_win;
#[cfg(windows)]
pub mod jeux_win;
#[cfg(windows)]
pub mod keys_win;
#[cfg(windows)]
pub mod overlay_win;
#[cfg(windows)]
pub mod tray_win;
#[cfg(windows)]
pub mod window;
pub mod clipboard;
pub mod translate;

// --- Journal (0.13.3) ------------------------------------------------------------------------
// Ce que l'application écrit va dans le fichier du journal par une poignée qu'elle garde
// elle-même (en 0.13.2, la sortie standard redirigée est restée vide sur certains lancements) ;
// sans journal (outils en console), sur la sortie standard.

static JOURNAL: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// Fichier du journal de l'application.
pub fn set_journal(f: std::fs::File) {
    if let Ok(mut j) = JOURNAL.lock() {
        *j = Some(f);
    }
}

thread_local! {
    /// Préfixe des lignes écrites par ce fil (« [WoW 2 Retail] » quand plusieurs jeux sont
    /// traduits, 0.15).
    static PREFIX: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Préfixe des lignes du journal écrites par ce fil (vide : aucun).
pub fn set_journal_prefix(p: &str) {
    PREFIX.with(|x| {
        let mut x = x.borrow_mut();
        if x.as_str() != p {
            x.clear();
            x.push_str(p);
        }
    });
}

/// Une ligne du journal (macro `say!`).
pub fn journal_line(s: &str) {
    use std::io::Write;
    let line = PREFIX.with(|p| {
        let p = p.borrow();
        if p.is_empty() { None } else { Some(format!("{p}{s}")) }
    });
    let s = line.as_deref().unwrap_or(s);
    if let Ok(mut j) = JOURNAL.lock() {
        if let Some(f) = j.as_mut() {
            let _ = writeln!(f, "{s}");
            let _ = f.flush();
            return;
        }
    }
    println!("{s}");
}

/// Comme `println!`, vers le journal de l'application quand il y en a un.
#[macro_export]
macro_rules! say {
    ($($t:tt)*) => {
        $crate::journal_line(&format!($($t)*))
    };
}

/// Programmes lancés par l'application (llama.cpp, curl, tar, reg, PowerShell) : sans fenêtre de
/// console, l'application n'en ayant pas elle-même (sinon une fenêtre noire s'ouvrirait).
pub fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}
