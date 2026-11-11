//! Régulateur : la traduction reste sur la carte graphique et se règle en direct sur ce que
//! le jeu tolère.
//!
//! Repère : la plus longue image de WoW de chaque demi-seconde (envoyée par l'addon). Une
//! réinitialisation du pilote, c'est la carte bloquée 2 s ; bien avant, WoW a des à-coups de
//! quelques centaines de millisecondes. Le régulateur garde ces à-coups sous un seuil tiré de
//! ce que WoW fait seul, loin des 2 s :
//!   - à-coup au-dessus du seuil pendant une traduction : un cran plus prudent ;
//!   - longtemps calme : un cran moins prudent ;
//!   - réinitialisation réelle du pilote : repli immédiat, réglage interdit sur ce PC.
//!
//! Crans 0 à 2 : réglages immédiats (priorité sur la carte, rafales plus courtes et pauses plus
//! longues, espacement des traductions). Cran 3 : serveur relancé avec des paquets et des lots
//! plus petits. Au-delà : repli (pack low, puis processeur).

use crate::burst::Bursts;
use crate::llama::{GpuPriority, Tuning};

/// À-coup toléré : au moins 100 ms, ou le double des pires images de WoW seul (95 %).
pub fn hitch_limit(baseline_worst_ms: &[f32]) -> f32 {
    let mut v: Vec<f32> = baseline_worst_ms.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let p95 = v.get(((v.len().max(1) - 1) as f32 * 0.95).round() as usize).copied().unwrap_or(0.0);
    (2.0 * p95).max(100.0)
}

/// Un cran de réglage.
#[derive(Clone, Debug, PartialEq)]
pub struct Rung {
    pub tuning: Tuning,
    /// Espacement minimal entre deux traductions, en secondes.
    pub pace: f64,
    pub bursts: Bursts,
}

pub fn ladder() -> Vec<Rung> {
    let gentle = Tuning::gentle();
    let idle = Tuning { priority: GpuPriority::Idle, ..gentle.clone() };
    vec![
        Rung { tuning: gentle.clone(), pace: 0.0, bursts: Bursts::gentle() },
        Rung { tuning: idle.clone(), pace: 2.0, bursts: Bursts { tokens: 8, pause_ms: 80 } },
        Rung { tuning: idle.clone(), pace: 3.0, bursts: Bursts { tokens: 4, pause_ms: 150 } },
        Rung {
            tuning: Tuning { nodes_per_submit: Some(4), ubatch: Some(32), ..idle },
            pace: 3.0,
            bursts: Bursts { tokens: 4, pause_ms: 150 },
        },
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Changer la priorité sur la carte, les rafales et l'espacement, sans relancer.
    Adjust { priority: GpuPriority, pace: f64, bursts: Bursts },
    /// Relancer le serveur avec ces réglages.
    Restart(Tuning),
    /// Plus de cran disponible : pack low, puis processeur.
    Fallback,
}

pub struct Regulator {
    pub limit_ms: f32,
    rungs: Vec<Rung>,
    pub level: usize,
    /// Demi-secondes depuis le dernier changement (pour ne pas changer en rafale).
    since_change: u32,
    /// Demi-secondes calmes de suite pendant des traductions.
    calm: u32,
    /// Plus haut cran atteint.
    pub peak: usize,
}

/// Demi-secondes à attendre après un changement avant d'en refaire un.
const SETTLE: u32 = 4;
/// Demi-secondes calmes (pendant des traductions) avant de redescendre d'un cran : 60 s.
const CALM_TO_RELAX: u32 = 120;

impl Regulator {
    pub fn new(limit_ms: f32) -> Regulator {
        Regulator { limit_ms, rungs: ladder(), level: 0, since_change: SETTLE, calm: 0, peak: 0 }
    }

    pub fn rung(&self) -> &Rung {
        &self.rungs[self.level.min(self.rungs.len() - 1)]
    }

    fn change(&mut self, to: usize) -> Action {
        let from = self.level;
        self.level = to;
        self.peak = self.peak.max(to);
        self.since_change = 0;
        self.calm = 0;
        if to >= self.rungs.len() {
            return Action::Fallback;
        }
        let (a, b) = (&self.rungs[from.min(self.rungs.len() - 1)], &self.rungs[to]);
        let same_server = a.tuning.nodes_per_submit == b.tuning.nodes_per_submit
            && a.tuning.ubatch == b.tuning.ubatch
            && a.tuning.graphics_queue == b.tuning.graphics_queue
            && a.tuning.gpu_layers == b.tuning.gpu_layers;
        if same_server {
            Action::Adjust { priority: b.tuning.priority, pace: b.pace, bursts: b.bursts }
        } else {
            Action::Restart(b.tuning.clone())
        }
    }

    /// Une mesure de l'addon (toutes les demi-secondes) : plus longue image, et traduction en
    /// cours ou non pendant cette demi-seconde.
    pub fn observe(&mut self, worst_ms: f32, translating: bool) -> Option<Action> {
        self.since_change = self.since_change.saturating_add(1);
        if self.level >= self.rungs.len() {
            return None;
        }
        if translating && worst_ms > self.limit_ms {
            self.calm = 0;
            if self.since_change >= SETTLE {
                return Some(self.change(self.level + 1));
            }
            return None;
        }
        if translating && worst_ms <= self.limit_ms * 0.5 {
            self.calm += 1;
            if self.calm >= CALM_TO_RELAX && self.level > 0 {
                return Some(self.change(self.level - 1));
            }
        }
        None
    }

    /// Réinitialisation réelle du pilote : repli immédiat (régulateur en direct, à venir).
    #[allow(dead_code)]
    pub fn driver_reset(&mut self) -> Action {
        self.change(self.rungs.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_follows_what_wow_does_alone() {
        assert_eq!(hitch_limit(&[]), 100.0);
        assert_eq!(hitch_limit(&[24.0, 25.0, 30.0]), 100.0);
        // WoW seul a déjà des images de 80 ms : seuil 160 ms.
        assert_eq!(hitch_limit(&[24.0, 80.0, 25.0, 26.0]), 160.0);
    }

    #[test]
    fn steps_up_on_hitches_and_relaxes_when_calm() {
        let mut r = Regulator::new(100.0);
        // Calme : rien.
        assert_eq!(r.observe(30.0, true), None);
        // À-coup hors traduction : ignoré (pas dû à nous).
        assert_eq!(r.observe(400.0, false), None);
        // À-coup pendant une traduction : priorité minimale, rafales de 8, traductions espacées de 2 s.
        assert_eq!(
            r.observe(180.0, true),
            Some(Action::Adjust { priority: GpuPriority::Idle, pace: 2.0, bursts: Bursts { tokens: 8, pause_ms: 80 } })
        );
        // Juste après : on laisse le réglage agir.
        assert_eq!(r.observe(180.0, true), None);
        let settle = |r: &mut Regulator| {
            for _ in 0..3 {
                r.observe(20.0, true);
            }
        };
        settle(&mut r);
        // Nouvel à-coup : rafales de 4, pauses de 150 ms, toujours sans relancer.
        assert!(matches!(r.observe(150.0, true), Some(Action::Adjust { bursts: Bursts { tokens: 4, .. }, .. })));
        settle(&mut r);
        // Encore : relance avec des paquets de 4 et des lots de 32.
        match r.observe(150.0, true) {
            Some(Action::Restart(t)) => assert_eq!((t.nodes_per_submit, t.ubatch), (Some(4), Some(32))),
            other => panic!("{other:?}"),
        }
        // 60 s calmes en traduisant : on redescend d'un cran (serveur différent : relance).
        let mut last = None;
        for _ in 0..CALM_TO_RELAX {
            if let Some(a) = r.observe(20.0, true) {
                last = Some(a);
            }
        }
        assert!(matches!(last, Some(Action::Restart(_))));
        assert_eq!(r.level, 2);
        // Réinitialisation du pilote : repli, puis plus rien.
        assert_eq!(r.driver_reset(), Action::Fallback);
        assert_eq!(r.observe(500.0, true), None);
        assert_eq!(r.peak, 4);
    }
}
