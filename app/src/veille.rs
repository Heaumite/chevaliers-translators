//! Surveillance de la boucle de traduction (0.17.3).
//!
//! Un fil à part, indépendant de la boucle principale :
//! - il écrit la présence de l'application (fichier relu par l'addon à chaque connexion et
//!   /reload) dans les dossiers des jeux traduits. Un blocage passager de la boucle, une capture
//!   relancée ou une fenêtre de WoW perdue quelques secondes ne la rendent plus périmée : l'addon
//!   annonçait « Chevaliers Translators n'est pas actif » alors que l'application tournait ;
//! - il note au journal une boucle principale qui ne répond plus (avec l'étape où elle s'est
//!   arrêtée), puis sa reprise.
//!
//! La présence n'est plus écrite si la boucle ne répond plus depuis `PRESENCE_GRACE` : une
//! application vraiment bloquée laisse alors le chat officiel au prochain /reload.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::Duration;

/// Boucle sans signe de vie depuis ce délai : notée au journal.
pub const STALL: Duration = Duration::from_secs(5);
/// Présence encore écrite pendant ce délai après le dernier signe de vie de la boucle, et pour un
/// dossier de jeu pendant ce délai après sa dernière session (capture relancée, fenêtre perdue).
pub const PRESENCE_GRACE: Duration = Duration::from_secs(60);
/// Présence réécrite à ce rythme (l'addon la juge périmée au-delà de 20 s).
pub const PRESENCE_EVERY: Duration = Duration::from_secs(2);
/// Capture sans signe de vie depuis ce délai : relancée.
pub const CAPTURE_STUCK: Duration = Duration::from_secs(10);

/// Étapes de la boucle principale, nommées au journal en cas de blocage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Start = 0,
    Games,
    Wait,
    Frames,
    Windows,
    Keep,
    Presence,
    Send,
    Stack,
    Screen,
    Cover,
    Panels,
    Render,
    Show,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Start => "démarrage",
            Phase::Games => "recherche des fenêtres de WoW",
            Phase::Wait => "attente des images du bandeau",
            Phase::Frames => "lecture des trames",
            Phase::Windows => "messages des fenêtres",
            Phase::Keep => "icône, téléchargements et mises à jour",
            Phase::Presence => "présence",
            Phase::Send => "envoi traduit (presse-papiers)",
            Phase::Stack => "rangement des fenêtres au-dessus de WoW",
            Phase::Screen => "écran du jeu",
            Phase::Cover => "cache du bandeau",
            Phase::Panels => "panneaux en jeu",
            Phase::Render => "rendu du chat traduit",
            Phase::Show => "affichage du chat traduit",
        }
    }

    fn from_u8(v: u8) -> Phase {
        const ALL: [Phase; 14] = [
            Phase::Start,
            Phase::Games,
            Phase::Wait,
            Phase::Frames,
            Phase::Windows,
            Phase::Keep,
            Phase::Presence,
            Phase::Send,
            Phase::Stack,
            Phase::Screen,
            Phase::Cover,
            Phase::Panels,
            Phase::Render,
            Phase::Show,
        ];
        ALL.get(v as usize).copied().unwrap_or(Phase::Start)
    }
}

static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static BEAT: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicU8 = AtomicU8::new(0);
/// Dossiers AddOns des jeux traduits, avec le moment (ms) de leur dernière session.
static DIRS: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());
static STARTED: Once = Once::new();

/// Millisecondes depuis le premier usage (horloge monotone).
pub fn ms() -> u64 {
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Signe de vie de la boucle principale, à l'étape donnée.
pub fn beat(phase: Phase) {
    BEAT.store(ms(), Ordering::Relaxed);
    PHASE.store(phase as u8, Ordering::Relaxed);
}

/// Dossier AddOns d'un jeu traduit : sa présence est écrite par le fil de surveillance.
pub fn claim(dir: &Path) {
    let now = ms();
    if let Ok(mut d) = DIRS.lock() {
        match d.iter_mut().find(|(p, _)| p == dir) {
            Some(e) => e.1 = now,
            None => d.push((dir.to_path_buf(), now)),
        }
    }
}

/// Boucle de traduction en marche : surveillance et présence actives jusqu'à la fin du garde.
pub struct Guard;

impl Guard {
    pub fn start() -> Guard {
        beat(Phase::Start);
        ACTIVE.store(true, Ordering::Relaxed);
        STARTED.call_once(|| {
            std::thread::spawn(watch_loop);
        });
        Guard
    }
}

impl Drop for Guard {
    /// Boucle finie (Quitter, WoW fermé) : plus de présence écrite par ce fil (le verrou attend
    /// une écriture en cours), l'application en reprend l'écriture ou la remet à zéro.
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Relaxed);
        if let Ok(mut d) = DIRS.lock() {
            d.clear();
        }
    }
}

/// Dossiers dont la présence est due : réclamés depuis moins de `grace`.
pub fn dirs_due(dirs: &[(PathBuf, u64)], now: u64, grace: Duration) -> Vec<PathBuf> {
    let g = grace.as_millis() as u64;
    let mut out: Vec<PathBuf> = dirs.iter().filter(|(_, t)| now.saturating_sub(*t) < g).map(|(p, _)| p.clone()).collect();
    out.sort();
    out.dedup();
    out
}

/// Suivi des blocages de la boucle principale : une ligne au début du blocage, puis toutes les
/// 30 s tant qu'il dure, et une à la reprise.
#[derive(Default, Debug)]
pub struct Stall {
    since_report: Option<Duration>,
    worst: Duration,
}

impl Stall {
    pub fn observe(&mut self, age: Duration, phase: Phase) -> Option<String> {
        if age >= STALL {
            self.worst = self.worst.max(age);
            let due = match self.since_report {
                None => true,
                Some(last) => age >= last + Duration::from_secs(30),
            };
            if due {
                self.since_report = Some(age);
                return Some(format!(
                    "[surveillance] l'application ne répond plus depuis {} s (étape : {})",
                    age.as_secs(),
                    phase.label()
                ));
            }
            return None;
        }
        self.since_report.take().map(|_| {
            let w = std::mem::take(&mut self.worst);
            format!("[surveillance] l'application répond de nouveau après {} s de blocage (étape : {})", w.as_secs(), phase.label())
        })
    }
}

fn watch_loop() {
    let mut stall = Stall::default();
    let mut last_presence = 0u64;
    let mut said_error = false;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if !ACTIVE.load(Ordering::Relaxed) {
            stall = Stall::default();
            continue;
        }
        let now = ms();
        let age = Duration::from_millis(now.saturating_sub(BEAT.load(Ordering::Relaxed)));
        let phase = Phase::from_u8(PHASE.load(Ordering::Relaxed));
        if let Some(line) = stall.observe(age, phase) {
            crate::say!("{} {line}", clock());
        }
        if now.saturating_sub(last_presence) < PRESENCE_EVERY.as_millis() as u64 || age >= PRESENCE_GRACE {
            continue;
        }
        last_presence = now;
        // Verrou gardé pendant l'écriture : la fin de la boucle attend qu'elle soit finie.
        let Ok(d) = DIRS.lock() else { continue };
        if !ACTIVE.load(Ordering::Relaxed) {
            continue;
        }
        for dir in dirs_due(&d, now, PRESENCE_GRACE) {
            if let Err(e) = crate::install::write_live_everywhere(&dir) {
                if !said_error {
                    said_error = true;
                    crate::say!("[présence] {e}");
                }
            }
        }
    }
}

/// Heure locale « HH:MM:SS » pour le journal.
pub fn clock() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut t = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut t) };
        format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
    }
    #[cfg(not(windows))]
    {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) % 86_400;
        format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stall_is_reported_once_then_every_30_s_then_its_end() {
        let mut s = Stall::default();
        assert_eq!(s.observe(Duration::from_secs(1), Phase::Wait), None);
        let first = s.observe(Duration::from_secs(5), Phase::Stack).unwrap();
        assert!(first.contains("depuis 5 s") && first.contains("rangement des fenêtres"), "{first}");
        assert_eq!(s.observe(Duration::from_secs(20), Phase::Stack), None);
        assert!(s.observe(Duration::from_secs(35), Phase::Stack).unwrap().contains("depuis 35 s"));
        assert_eq!(s.observe(Duration::from_secs(40), Phase::Stack), None);
        let back = s.observe(Duration::from_millis(10), Phase::Wait).unwrap();
        assert!(back.contains("de nouveau après 40 s"), "{back}");
        // Fini : plus rien jusqu'au prochain blocage.
        assert_eq!(s.observe(Duration::from_millis(10), Phase::Wait), None);
        assert!(s.observe(Duration::from_secs(6), Phase::Render).unwrap().contains("depuis 6 s"));
    }

    #[test]
    fn presence_is_due_for_recent_games_only() {
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");
        let dirs = vec![(a.clone(), 100_000), (b.clone(), 10_000), (a.clone(), 90_000)];
        assert_eq!(dirs_due(&dirs, 105_000, PRESENCE_GRACE), vec![a.clone()]);
        assert_eq!(dirs_due(&dirs, 69_000, PRESENCE_GRACE), vec![a, b]);
        assert!(dirs_due(&dirs, 200_000, PRESENCE_GRACE).is_empty());
    }

    #[test]
    fn phases_round_trip_and_have_names() {
        for v in 0..14u8 {
            let p = Phase::from_u8(v);
            assert_eq!(p as u8, v);
            assert!(!p.label().is_empty());
        }
        assert_eq!(Phase::from_u8(200), Phase::Start);
    }
}
