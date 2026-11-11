//! Mesures prises pendant le jeu, sans aucun test : ce que le PC du joueur demande vraiment.
//!
//! Toutes les 5 s pendant que la traduction tourne et que WoW est ouvert : mémoire vidéo prise
//! par WoW, mémoire vidéo qu'un modèle pourrait occuper au pire moment (libre + celle du modèle
//! chargé sur la carte), temps passé en instance (donjon, raid, champ de bataille, d'après
//! l'addon), rythme du chat reçu. Cumulées d'une session à l'autre par conditions (version du
//! jeu et taille d'écran) dans `mesures.tsv`. Elles servent à proposer un modèle supérieur
//! (module `conseil`) ; elles ne pèsent rien sur le jeu (un relevé que l'application faisait
//! déjà pour surveiller la mémoire vidéo).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::hw::GIB;

/// Temps de jeu observé avant toute proposition.
pub const MIN_MINUTES: f64 = 30.0;
/// Temps en instance au-delà duquel les pics de WoW sont jugés vus (raids, donjons).
pub const INSTANCE_MINUTES: f64 = 10.0;
/// Un relevé compte au plus pour cette durée (application en pause, PC en veille).
const MAX_STEP: Duration = Duration::from_secs(15);
/// Fenêtre du rythme du chat.
const CHAT_WINDOW: Duration = Duration::from_secs(300);

/// Ce qui a été mesuré dans des conditions données.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    /// « _retail_ 3840x1600 ».
    pub key: String,
    pub minutes: f64,
    /// Dont en instance (donjon, raid, champ de bataille, arène, scénario).
    pub instance_minutes: f64,
    /// Mémoire vidéo de WoW, au plus haut.
    pub wow_peak: u64,
    /// Mémoire vidéo qu'un modèle aurait pu prendre au pire moment : libre, plus celle du modèle
    /// de traduction s'il était sur la carte.
    pub room_low: u64,
    /// Messages reçus à traduire par minute, au plus haut sur 5 min.
    pub chat_peak: f64,
    /// Dernière mise à jour (secondes depuis 1970).
    pub updated: u64,
}

impl Observed {
    /// Marge gardée sur la mémoire mesurée : faible si des instances ont été vues (les pics de
    /// WoW sont dans la mesure), sinon 1 Go ou 20 % de WoW (un premier raid fera plus).
    pub fn margin(&self) -> u64 {
        if self.instance_minutes >= INSTANCE_MINUTES {
            GIB / 4
        } else {
            GIB.max(self.wow_peak / 5)
        }
    }

    fn line(&self) -> String {
        format!(
            "{}\t{:.1}\t{:.1}\t{}\t{}\t{:.1}\t{}",
            self.key, self.minutes, self.instance_minutes, self.wow_peak, self.room_low, self.chat_peak, self.updated
        )
    }

    fn parse(line: &str) -> Option<Observed> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 || f[0].starts_with('#') {
            return None;
        }
        Some(Observed {
            key: f[0].to_string(),
            minutes: f[1].parse().ok()?,
            instance_minutes: f[2].parse().ok()?,
            wow_peak: f[3].parse().ok()?,
            room_low: f[4].parse().ok()?,
            chat_peak: f[5].parse().ok()?,
            updated: f[6].parse().ok()?,
        })
    }
}

/// Clé des conditions : version du jeu et taille d'écran.
pub fn key(flavor: Option<&str>, size: Option<(u32, u32)>) -> String {
    let f = flavor.unwrap_or("_inconnue_");
    match size {
        Some((w, h)) => format!("{f} {w}x{h}"),
        None => format!("{f} ?"),
    }
}

pub fn file() -> PathBuf {
    crate::fonts::cache_root().join("mesures.tsv")
}

const HEADER: &str = "# Mesures prises pendant le jeu (Chevaliers Translators) : conditions, minutes, dont en instance, WoW au plus haut (octets), place pour un modèle au plus bas (octets), messages par minute au plus haut, date\n";

pub fn load_from(path: &std::path::Path, key: &str) -> Option<Observed> {
    std::fs::read_to_string(path).ok()?.lines().filter_map(Observed::parse).find(|o| o.key == key)
}

pub fn load(key: &str) -> Option<Observed> {
    load_from(&file(), key)
}

pub fn save_to(path: &std::path::Path, o: &Observed) -> Result<(), String> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::from(HEADER);
    for l in old.lines().filter(|l| Observed::parse(l).is_some_and(|x| x.key != o.key)) {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&o.line());
    out.push('\n');
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(path, out).map_err(|e| format!("{} : {e}", path.display()))
}

pub fn save(o: &Observed) -> Result<(), String> {
    save_to(&file(), o)
}

/// Relevés de la session en cours, versés dans `mesures.tsv` toutes les minutes.
#[derive(Debug)]
pub struct Session {
    pub key: String,
    minutes: f64,
    instance_minutes: f64,
    wow_peak: u64,
    room_low: Option<u64>,
    chat: VecDeque<Instant>,
    chat_peak: f64,
    started: Instant,
    last: Option<Instant>,
}

impl Session {
    pub fn new(key: String, now: Instant) -> Session {
        Session {
            key,
            minutes: 0.0,
            instance_minutes: 0.0,
            wow_peak: 0,
            room_low: None,
            chat: VecDeque::new(),
            chat_peak: 0.0,
            started: now,
            last: None,
        }
    }

    /// Un relevé : mémoire vidéo libre, celle de WoW si connue, celle du modèle chargé sur la
    /// carte (0 s'il est sur le processeur), instance en cours.
    pub fn sample(&mut self, now: Instant, free: u64, wow: Option<u64>, model_on_gpu: u64, in_instance: bool) {
        if let Some(prev) = self.last {
            let dt = now.saturating_duration_since(prev).min(MAX_STEP).as_secs_f64() / 60.0;
            self.minutes += dt;
            if in_instance {
                self.instance_minutes += dt;
            }
        }
        self.last = Some(now);
        if let Some(w) = wow {
            self.wow_peak = self.wow_peak.max(w);
        }
        let room = free + model_on_gpu;
        self.room_low = Some(self.room_low.map_or(room, |r| r.min(room)));
    }

    /// Relevé interrompu (WoW fermé, traduction arrêtée) : le temps d'ici au prochain ne compte pas.
    pub fn pause(&mut self) {
        self.last = None;
    }

    /// Un message reçu à traduire.
    pub fn message(&mut self, now: Instant) {
        self.chat.push_back(now);
        while self.chat.front().is_some_and(|t| now.saturating_duration_since(*t) > CHAT_WINDOW) {
            self.chat.pop_front();
        }
        // Rythme sur la fenêtre (ou depuis le début de la session si elle est plus courte, au
        // moins une minute pour ne pas compter une rafale isolée comme un rythme).
        let span = now.saturating_duration_since(self.started).min(CHAT_WINDOW).max(Duration::from_secs(60));
        let rate = self.chat.len() as f64 / (span.as_secs_f64() / 60.0);
        self.chat_peak = self.chat_peak.max(rate);
    }

    /// Verse la session dans les mesures déjà gardées et remet ses compteurs à zéro (les pics
    /// restent, la session continue).
    pub fn merge_into(&mut self, prev: Option<Observed>, when: u64) -> Option<Observed> {
        let room = self.room_low?;
        let o = match prev.filter(|p| p.key == self.key) {
            Some(p) => Observed {
                key: self.key.clone(),
                minutes: p.minutes + self.minutes,
                instance_minutes: p.instance_minutes + self.instance_minutes,
                wow_peak: p.wow_peak.max(self.wow_peak),
                room_low: p.room_low.min(room),
                chat_peak: p.chat_peak.max(self.chat_peak),
                updated: when,
            },
            None => Observed {
                key: self.key.clone(),
                minutes: self.minutes,
                instance_minutes: self.instance_minutes,
                wow_peak: self.wow_peak,
                room_low: room,
                chat_peak: self.chat_peak,
                updated: when,
            },
        };
        self.minutes = 0.0;
        self.instance_minutes = 0.0;
        Some(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_accumulates_and_merges() {
        let t0 = Instant::now();
        let mut s = Session::new(key(Some("_forever_"), Some((3840, 1600))), t0);
        assert_eq!(s.key, "_forever_ 3840x1600");
        // 10 min de relevés toutes les 5 s, dont 4 min en instance ; WoW grossit en instance.
        for i in 0..=120u64 {
            let inst = (72..120).contains(&i);
            let wow = if inst { 7 * GIB } else { 5 * GIB };
            let free = if inst { 2 * GIB } else { 4 * GIB };
            s.sample(t0 + Duration::from_secs(i * 5), free, Some(wow), GIB, inst);
        }
        // Pause (PC en veille 1 h) : au plus 15 s comptées.
        s.sample(t0 + Duration::from_secs(605 + 3600), 4 * GIB, Some(5 * GIB), GIB, false);
        for k in 0..30u64 {
            s.message(t0 + Duration::from_secs(300 + k * 4));
        }
        let o = s.merge_into(None, 1).unwrap();
        assert!((o.minutes - 10.25).abs() < 0.01, "{}", o.minutes);
        assert!((o.instance_minutes - 4.0).abs() < 0.01, "{}", o.instance_minutes);
        assert_eq!((o.wow_peak, o.room_low), (7 * GIB, 3 * GIB));
        assert!(o.chat_peak >= 6.0 && o.chat_peak <= 31.0, "{}", o.chat_peak);
        // Compteurs remis à zéro, pics gardés ; seconde session versée par-dessus la première.
        s.sample(t0 + Duration::from_secs(4300), 5 * GIB, Some(4 * GIB), 0, false);
        s.sample(t0 + Duration::from_secs(4310), 5 * GIB, Some(4 * GIB), 0, false);
        let o2 = s.merge_into(Some(o.clone()), 2).unwrap();
        // 15 s au plus pour l’écart depuis le dernier relevé, puis 10 s.
        assert!((o2.minutes - o.minutes - 25.0 / 60.0).abs() < 0.01, "{}", o2.minutes - o.minutes);
        assert_eq!((o2.wow_peak, o2.room_low, o2.updated), (7 * GIB, 3 * GIB, 2));
        // Marge : instances vues, 0,25 Go ; sinon 1 Go ou 20 % de WoW.
        // 4 min en instance (moins de 10) : 1 Go ou 20 % de WoW (7 Go : 1,4 Go).
        assert_eq!(o2.margin(), 7 * GIB / 5);
        assert_eq!(Observed { instance_minutes: 12.0, ..o2.clone() }.margin(), GIB / 4);
        let calm = Observed { instance_minutes: 0.0, wow_peak: 12 * GIB, ..o2.clone() };
        assert_eq!(calm.margin(), 12 * GIB / 5);
    }

    #[test]
    fn file_keeps_one_line_per_conditions() {
        let dir = std::env::temp_dir().join(format!("ct-mesures-{}", std::process::id()));
        let path = dir.join("mesures.tsv");
        let a = Observed { key: "_retail_ 3840x1600".into(), minutes: 42.5, instance_minutes: 12.0, wow_peak: 11 * GIB, room_low: 3 * GIB, chat_peak: 14.2, updated: 9 };
        let b = Observed { key: "_forever_ 3840x1600".into(), minutes: 5.0, ..a.clone() };
        save_to(&path, &a).unwrap();
        save_to(&path, &b).unwrap();
        save_to(&path, &Observed { minutes: 50.0, ..a.clone() }).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().filter(|l| !l.starts_with('#')).count(), 2, "{text}");
        assert_eq!(load_from(&path, "_retail_ 3840x1600").unwrap().minutes, 50.0);
        assert_eq!(load_from(&path, "_forever_ 3840x1600").unwrap(), b);
        assert!(load_from(&path, "_classic_era_ 1920x1080").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
