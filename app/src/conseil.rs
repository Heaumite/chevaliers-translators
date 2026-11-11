//! Proposition d'un modèle supérieur d'après les mesures prises pendant le jeu (0.14).
//!
//! La règle (module `plan`) choisit d'office le meilleur modèle sûr ou probable ; la mémoire
//! vidéo se gère en jeu (module `gouverneur`, 0.14.2 : plus de réserve fixe, descente près de la
//! saturation, remontée quand la place revient). Restent écartés par la règle les modèles qui ont
//! déjà fait réinitialiser le pilote graphique par Windows ici : les mesures (module `mesures`)
//! peuvent montrer qu'ils tiendraient en mémoire. Un tel modèle n'est jamais activé seul : il est
//! proposé, avec un avertissement, par un panneau sur le jeu hors combat ; `/chevaliers monter`
//! l'essaie (repli automatique en cas de souci), `/chevaliers non` ne le propose plus.

use std::path::{Path, PathBuf};

use crate::catalog::{self, Model, Place, Verdict};
use crate::hw::{gib, GIB};
use crate::mesures::{Observed, MIN_MINUTES};
use crate::plan::{Candidate, Plan};

/// Un modèle proposé.
#[derive(Clone, Debug)]
pub struct Proposal {
    pub model: &'static Model,
    pub place: Place,
    pub footprint: u64,
    /// Taille à télécharger s'il n'est pas sur le PC.
    pub download: Option<u64>,
    /// Pourquoi il tiendrait (mesures).
    pub reason: String,
    /// Avertissement montré avec la proposition.
    pub warning: String,
}

impl Proposal {
    /// Clé de refus : modèle et place.
    pub fn key(&self) -> String {
        format!("{}:{}", self.model.id, self.place.code())
    }
}

fn minutes(m: f64) -> String {
    let m = m.round() as u64;
    if m >= 60 {
        format!("{} h {:02}", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// Modèle à proposer, le meilleur d'abord, ou rien. `active` : modèle en service ; `refused` :
/// modèles écartés par le joueur ; `installed` : présents sur le PC.
pub fn propose(
    plan: &Plan,
    active: Option<&Candidate>,
    obs: &Observed,
    refused: &dyn Fn(&str) -> bool,
    installed: &dyn Fn(&Model) -> bool,
) -> Option<Proposal> {
    if obs.minutes < MIN_MINUTES {
        return None;
    }
    let q0 = active.map_or(0, |a| a.model.quality);
    let in_chain = |c: &Candidate| plan.chain.iter().any(|&i| std::ptr::eq(&plan.candidates[i], c));
    let mut list: Vec<&Candidate> = plan
        .candidates
        .iter()
        .filter(|c| c.model.quality > q0 && c.possible && !in_chain(c))
        .collect();
    list.sort_by(|a, b| b.model.quality.cmp(&a.model.quality).then((a.place != Place::Gpu).cmp(&(b.place != Place::Gpu))));
    for c in list {
        let key = format!("{}:{}", c.model.id, c.place.code());
        if refused(&key) {
            continue;
        }
        // Manque de mémoire constaté ici : la mesure l'a déjà dit.
        if matches!(c.banned, Some((Verdict::Memory, _))) {
            continue;
        }
        // Trop lent pour le chat : rien à gagner.
        if c.latency > catalog::MAX_LATENCY * 1.25 || c.capacity < obs.chat_peak.min(crate::plan::DESIGN_RATE) * 0.75 {
            continue;
        }
        let reason = match c.place {
            Place::Gpu => {
                if plan.card.is_none() {
                    continue;
                }
                let margin = obs.margin();
                let room = obs.room_low.saturating_sub(margin);
                let sat = plan.card.as_ref().map_or(0, |cd| crate::gouverneur::saturation(cd.dedicated));
                if c.footprint + sat > room {
                    continue;
                }
                format!(
                    "Mesuré pendant {} de jeu{} : au pire moment, {} de mémoire vidéo restaient pour un modèle ; il en prend {} (marge de {} en plus) ; près de la saturation, la traduction redescendrait seule.",
                    minutes(obs.minutes),
                    if obs.instance_minutes >= crate::mesures::INSTANCE_MINUTES {
                        format!(", dont {} en instance", minutes(obs.instance_minutes))
                    } else {
                        String::new()
                    },
                    gib(obs.room_low),
                    gib(c.footprint),
                    gib(margin)
                )
            }
            Place::Cpu => format!(
                "Sur le processeur : {:.1} s par message prévu, mémoire vive suffisante ; mesuré pendant {} de jeu.",
                c.latency,
                minutes(obs.minutes)
            )
            .replace('.', ","),
        };
        let mut warning = String::from(
            "Avertissement : la règle ne le retient pas pour ton PC. Il pourrait provoquer des plantages (réinitialisation du pilote graphique par Windows) ou ralentir le jeu ; en cas de souci, la traduction revient seule au modèle actuel.",
        );
        if let Some((Verdict::Crash, what)) = &c.banned {
            warning += &format!(" Il en a déjà provoqué un sur ce PC ({what}).");
        }
        return Some(Proposal {
            model: c.model,
            place: c.place,
            footprint: c.footprint,
            download: (!installed(c.model)).then_some(c.model.size),
            reason,
            warning,
        });
    }
    None
}

/// Lignes du panneau de proposition (affiché sur le jeu, hors combat).
pub fn panel_rows(p: &Proposal) -> Vec<crate::panel::Row> {
    use crate::panel::{Row, DIM, TEXT, TITLE};
    let mut rows = vec![
        Row { text: "Modèle de traduction supérieur possible".into(), color: TITLE, title: true },
        Row { text: format!("{} {} : meilleure traduction.", p.model.name, p.place.label()), color: TEXT, title: false },
        Row { text: p.reason.clone(), color: TEXT, title: false },
        Row { text: p.warning.clone(), color: [230, 160, 70], title: false },
    ];
    if let Some(size) = p.download {
        rows.push(Row { text: format!("Téléchargement de {} avant l'essai (débit bridé pendant le jeu).", catalog::go(size)), color: DIM, title: false });
    }
    rows.push(Row { text: "/chevaliers monter : l'essayer   ·   /chevaliers non : ne plus le proposer".into(), color: DIM, title: false });
    rows
}

// --- Choix et refus du joueur, par conditions (version du jeu, taille d'écran) -----------------

fn choices_file() -> PathBuf {
    crate::fonts::cache_root().join("choix.tsv")
}

fn refusals_file() -> PathBuf {
    crate::fonts::cache_root().join("propositions-refusees.tsv")
}

fn lines(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').map(String::from).collect())
        .collect()
}

fn write_lines(path: &Path, header: &str, rows: &[Vec<String>]) {
    let mut out = String::from(header);
    for r in rows {
        out += &r.join("\t");
        out.push('\n');
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, out);
}

/// Modèle choisi par le joueur (proposition acceptée) pour ces conditions.
pub fn choice_in(path: &Path, key: &str) -> Option<(&'static Model, Place)> {
    lines(path).into_iter().find(|r| r.len() >= 3 && r[0] == key).and_then(|r| {
        let m = catalog::by_id(&r[1])?;
        let p = if r[2] == "cpu" { Place::Cpu } else { Place::Gpu };
        Some((m, p))
    })
}

pub fn choice(key: &str) -> Option<(&'static Model, Place)> {
    choice_in(&choices_file(), key)
}

/// Retient (Some) ou oublie (None, après un échec) le choix du joueur pour ces conditions.
pub fn set_choice_in(path: &Path, key: &str, what: Option<(&Model, Place)>) {
    let mut rows: Vec<Vec<String>> = lines(path).into_iter().filter(|r| r.first().map(String::as_str) != Some(key)).collect();
    if let Some((m, p)) = what {
        rows.push(vec![key.to_string(), m.id.to_string(), p.code().to_string(), catalog::now_secs().to_string()]);
    }
    write_lines(path, "# Modèles choisis par le joueur (propositions acceptées) : conditions, modèle, place, date\n", &rows);
}

pub fn set_choice(key: &str, what: Option<(&Model, Place)>) {
    set_choice_in(&choices_file(), key, what)
}

pub fn refused_in(path: &Path, key: &str, what: &str) -> bool {
    lines(path).iter().any(|r| r.len() >= 2 && r[0] == key && r[1] == what)
}

pub fn refused(key: &str, what: &str) -> bool {
    refused_in(&refusals_file(), key, what)
}

pub fn refuse_in(path: &Path, key: &str, what: &str) {
    let mut rows = lines(path);
    if !rows.iter().any(|r| r.len() >= 2 && r[0] == key && r[1] == what) {
        rows.push(vec![key.to_string(), what.to_string(), catalog::now_secs().to_string()]);
    }
    write_lines(path, "# Propositions écartées par le joueur (/chevaliers non) : conditions, modèle:place, date\n", &rows);
}

pub fn refuse(key: &str, what: &str) {
    refuse_in(&refusals_file(), key, what)
}

/// Chaîne de repli avec le choix du joueur en tête (s'il est permis : pas de manque de mémoire
/// constaté). Renvoie vrai si la chaîne a changé.
pub fn apply_choice(plan: &mut Plan, choice: Option<(&'static Model, Place)>) -> bool {
    let Some((m, p)) = choice else { return false };
    let Some(i) = plan.candidates.iter().position(|c| c.model.id == m.id && c.place == p) else { return false };
    if matches!(plan.candidates[i].banned, Some((Verdict::Memory, _))) {
        return false;
    }
    plan.chain.retain(|&k| k != i);
    plan.chain.insert(0, i);
    true
}

#[allow(dead_code)]
const _GIB: u64 = GIB;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{estimate, Conditions};
    use crate::catalog::Trials;

    fn obs(minutes: f64, room: u64, inst: f64, wow: u64) -> Observed {
        Observed { key: "_retail_ 3840x1600".into(), minutes, instance_minutes: inst, wow_peak: wow, room_low: room, chat_peak: 10.0, updated: 1 }
    }

    #[test]
    fn measured_room_unlocks_a_better_model() {
        // RTX 3060 12 Go : le 7B ne tient pas à côté de WoW compté 7 Go (pas dans la chaîne).
        let h = crate::plan::tests_support::pc("NVIDIA GeForce RTX 3060", 0x10DE, 12, 6);
        let none = |_: &str| None;
        let p = estimate(&h, &none, &Trials::new(), Conditions::worst());
        let active = p.best();
        assert_eq!(active.unwrap().model.id, "translategemma-4b");
        let all = |_: &Model| true;
        let no = |_: &str| false;
        // Trop peu de jeu observé : rien.
        assert!(propose(&p, active, &obs(10.0, 9 * GIB, 0.0, 4 * GIB), &no, &all).is_none());
        // 7B hors de la chaîne (trop peu de chat suivi d'après l'estimation). 45 min, dont 15 en
        // instance, WoW léger : 5,2 Go de place au pire moment ; le 7B prend 4,3 Go, plus le seuil
        // de saturation (0,5 Go) et la marge des mesures (0,25 Go) -> il tient.
        let pr = propose(&p, active, &obs(45.0, 52 * GIB / 10, 15.0, 3 * GIB), &no, &all).expect("proposition");
        assert_eq!((pr.model.id, pr.place), ("salamandrata-7b", Place::Gpu));
        assert!(pr.reason.contains("45 min de jeu, dont 15 min en instance"), "{}", pr.reason);
        assert!(pr.warning.contains("plantages"), "{}", pr.warning);
        assert!(pr.download.is_none());
        // Sans instance vue : marge des mesures de 1 Go, il ne tient plus.
        let calm = propose(&p, active, &obs(45.0, 52 * GIB / 10, 0.0, 3 * GIB), &no, &all);
        assert!(calm.is_none(), "{:?}", calm.map(|c| (c.model.id, c.place, c.footprint, c.reason)));
        // Refusé : plus proposé ; pas sur le PC : taille à télécharger.
        let refused = |k: &str| k == "salamandrata-7b:gpu";
        assert!(propose(&p, active, &obs(45.0, 52 * GIB / 10, 15.0, 3 * GIB), &refused, &all).is_none());
        let missing = |m: &Model| m.id != "salamandrata-7b";
        let pr = propose(&p, active, &obs(45.0, 52 * GIB / 10, 15.0, 3 * GIB), &no, &missing).unwrap();
        assert!(pr.download.is_some());
        let rows = panel_rows(&pr);
        assert!(rows.iter().any(|r| r.text.contains("/chevaliers monter")));
        assert!(rows.iter().any(|r| r.text.starts_with("Téléchargement de")));
    }

    #[test]
    fn choices_and_refusals_are_kept_per_conditions() {
        let dir = std::env::temp_dir().join(format!("ct-conseil-{}", std::process::id()));
        let (c, r) = (dir.join("choix.tsv"), dir.join("refus.tsv"));
        let m = catalog::by_id("translategemma-4b").unwrap();
        set_choice_in(&c, "_retail_ 3840x1600", Some((m, Place::Gpu)));
        set_choice_in(&c, "_forever_ 3840x1600", Some((m, Place::Cpu)));
        assert_eq!(choice_in(&c, "_retail_ 3840x1600").map(|(m, p)| (m.id, p)), Some(("translategemma-4b", Place::Gpu)));
        set_choice_in(&c, "_retail_ 3840x1600", None);
        assert!(choice_in(&c, "_retail_ 3840x1600").is_none());
        assert_eq!(choice_in(&c, "_forever_ 3840x1600").map(|(_, p)| p), Some(Place::Cpu));
        refuse_in(&r, "_retail_ 3840x1600", "salamandrata-7b:gpu");
        refuse_in(&r, "_retail_ 3840x1600", "salamandrata-7b:gpu");
        assert!(refused_in(&r, "_retail_ 3840x1600", "salamandrata-7b:gpu"));
        assert!(!refused_in(&r, "_forever_ 3840x1600", "salamandrata-7b:gpu"));
        assert_eq!(std::fs::read_to_string(&r).unwrap().lines().filter(|l| !l.starts_with('#')).count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accepted_choice_leads_the_chain() {
        let h = crate::plan::tests_support::pc("NVIDIA GeForce RTX 3060", 0x10DE, 12, 6);
        let none = |_: &str| None;
        let mut p = estimate(&h, &none, &Trials::new(), Conditions::worst());
        let m7 = catalog::by_id("salamandrata-7b").unwrap();
        assert!(apply_choice(&mut p, Some((m7, Place::Gpu))));
        assert_eq!(p.best().unwrap().model.id, "salamandrata-7b");
        // Puis la chaîne de la règle, en repli.
        assert_eq!(p.step(1).unwrap().model.id, "translategemma-4b");
        assert!(!apply_choice(&mut p, None));
    }
}
