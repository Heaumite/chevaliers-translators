//! Mémoire vidéo pendant le jeu (0.14.2) : plus de réserve fixe de 1 Go.
//!
//! Le meilleur modèle de la chaîne qui tient dans la mémoire vidéo libre est chargé. Quand la
//! mémoire vidéo approche de la saturation (moins de 4 % de la carte libres, au moins 512 Mo, deux
//! relevés de suite), il redescend d'une étape, même en combat : sa mémoire est rendue au jeu tout
//! de suite. Le seuil reste au-dessus des 0,3 Go qui restaient lors de la réinitialisation du
//! pilote graphique du 09/11/2026 (TranslateGemma 12B chargé pendant le jeu). Il remonte hors combat quand la place revient (assez de place pour le modèle
//! supérieur et 256 Mo au-dessus du seuil, une minute de suite) ; un modèle qui vient de saturer
//! attend avant d'être repris (3 min, puis 6, 12... jusqu'à 30 min), sauf si les réglages
//! graphiques de WoW changent : tout est alors réévalué.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const MIB: u64 = 1 << 20;
/// Place revenue pendant ce temps avant de remonter.
pub const STABLE: Duration = Duration::from_secs(60);
const HOLD_FIRST: Duration = Duration::from_secs(180);
const HOLD_MAX: Duration = Duration::from_secs(1800);

/// Mémoire vidéo presque pleine : sous ce seuil de mémoire libre, le modèle redescend.
pub fn saturation(dedicated: u64) -> u64 {
    (dedicated / 25).max(512 * MIB)
}

/// Place à avoir en plus de l'empreinte du modèle supérieur pour y remonter.
pub fn headroom(dedicated: u64) -> u64 {
    saturation(dedicated) + 256 * MIB
}

/// Une étape de la chaîne de repli, telle que le gouverneur la voit.
#[derive(Clone, Debug)]
pub struct Step {
    pub label: String,
    pub gpu: bool,
    pub footprint: u64,
    /// Installée sur le PC.
    pub usable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Stay,
    /// Redescendre (étape plus légère, ou aucune si plus rien ne tient).
    Down(Option<usize>),
    /// Remonter à cette étape.
    Up(usize),
}

pub struct Governor {
    saturation: u64,
    headroom: u64,
    low: u32,
    /// Étape -> (pas avant, nombre de saturations).
    holds: HashMap<String, (Instant, u32)>,
    up: Option<(usize, Instant)>,
}

impl Governor {
    pub fn new(dedicated: u64) -> Governor {
        Governor { saturation: saturation(dedicated), headroom: headroom(dedicated), low: 0, holds: HashMap::new(), up: None }
    }

    pub fn saturation(&self) -> u64 {
        self.saturation
    }

    fn held(&self, s: &Step, now: Instant) -> bool {
        self.holds.get(&s.label).is_some_and(|(until, _)| now < *until)
    }

    /// L'étape a saturé la mémoire vidéo (en jeu ou au chargement) : mise de côté un moment,
    /// plus longtemps à chaque fois.
    pub fn saturated(&mut self, label: &str, now: Instant) -> Duration {
        let n = self.holds.get(label).map_or(0, |(_, n)| *n) + 1;
        let d = HOLD_FIRST.saturating_mul(1 << (n - 1).min(4)).min(HOLD_MAX);
        self.holds.insert(label.to_string(), (now + d, n));
        self.up = None;
        d
    }

    /// Réglages graphiques de WoW changés : la place est réévaluée, aucune étape mise de côté.
    pub fn graphics_changed(&mut self) {
        self.holds.clear();
        self.up = None;
        self.low = 0;
    }

    /// Prochaine étape plus légère que `pos`, installée et pas mise de côté.
    pub fn lighter(&self, steps: &[Step], pos: usize, now: Instant) -> Option<usize> {
        let cur = steps.get(pos)?;
        (pos + 1..steps.len()).find(|&i| {
            let s = &steps[i];
            s.usable && !self.held(s, now) && (!s.gpu || !cur.gpu || s.footprint < cur.footprint)
        })
    }

    /// Meilleure étape qui tient maintenant : `avail`, la mémoire vidéo libre en comptant celle du
    /// modèle chargé ; sur le processeur, toujours (mémoire vive vérifiée par la règle).
    pub fn best_fit(&self, steps: &[Step], avail: Option<u64>, now: Instant) -> Option<usize> {
        steps.iter().position(|s| {
            s.usable && !self.held(s, now) && (!s.gpu || avail.is_none_or(|a| s.footprint + self.headroom <= a))
        })
    }

    /// Un relevé de mémoire vidéo libre (`free`), étape chargée `pos`.
    pub fn sample(&mut self, steps: &[Step], pos: Option<usize>, free: u64, combat: bool, now: Instant) -> Move {
        let cur = pos.and_then(|p| steps.get(p));
        let on_gpu = cur.is_some_and(|s| s.gpu);
        if on_gpu && free < self.saturation {
            self.low += 1;
        } else {
            self.low = 0;
        }
        if self.low >= 2 {
            self.low = 0;
            let (p, c) = (pos.unwrap(), cur.unwrap());
            self.saturated(&c.label.clone(), now);
            return Move::Down(self.lighter(steps, p, now));
        }
        if combat {
            self.up = None;
            return Move::Stay;
        }
        let avail = free + cur.filter(|s| s.gpu).map_or(0, |s| s.footprint);
        let limit = pos.unwrap_or(steps.len());
        let target = self.best_fit(steps, Some(avail), now).filter(|&t| t < limit);
        match (target, self.up) {
            (Some(t), Some((u, since))) if t == u => {
                if now.saturating_duration_since(since) >= STABLE {
                    self.up = None;
                    return Move::Up(t);
                }
            }
            (Some(t), _) => self.up = Some((t, now)),
            (None, _) => self.up = None,
        }
        Move::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn steps() -> Vec<Step> {
        let s = |label: &str, gpu: bool, fp: u64| Step { label: label.into(), gpu, footprint: fp, usable: true };
        vec![
            s("SalamandraTA 7B sur la carte", true, 46 * GIB / 10),
            s("TranslateGemma 4B sur la carte", true, 285 * GIB / 100),
            s("EuroLLM 1.7B sur la carte", true, 115 * GIB / 100),
            s("EuroLLM 1.7B sur le processeur", false, 0),
        ]
    }

    #[test]
    fn steps_down_near_saturation_and_back_up_when_room_returns() {
        let st = steps();
        let mut g = Governor::new(16 * GIB);
        assert_eq!(g.saturation(), 16 * GIB / 25);
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Au départ : 3,8 Go libres -> le 4B (2,85 Go + 0,89 Go) tient, pas le 7B.
        assert_eq!(g.best_fit(&st, Some(38 * GIB / 10), t0), Some(1));
        // Le 4B chargé ; WoW grossit : 0,2 Go libres deux fois de suite -> 1.7B sur la carte.
        assert_eq!(g.sample(&st, Some(1), GIB / 5, false, at(5)), Move::Stay);
        assert_eq!(g.sample(&st, Some(1), GIB / 5, true, at(7)), Move::Down(Some(2)));
        // Le 4B est mis de côté 3 min, même si la place revient tout de suite.
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, false, at(10)), Move::Stay);
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, false, at(80)), Move::Stay);
        // Après 3 min, la place (3 Go libres + 1,15 Go du modèle chargé) suffit au 4B pendant 1 min.
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, false, at(190)), Move::Stay);
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, true, at(230)), Move::Stay, "jamais en combat");
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, false, at(240)), Move::Stay);
        assert_eq!(g.sample(&st, Some(2), 3 * GIB, false, at(300)), Move::Up(1));
        // Nouvelle saturation : 6 min de côté cette fois.
        g.sample(&st, Some(1), GIB / 10, false, at(400));
        assert_eq!(g.sample(&st, Some(1), GIB / 10, false, at(402)), Move::Down(Some(2)));
        assert_eq!(g.best_fit(&st, Some(6 * GIB), at(402 + 300)), Some(0), "7B jamais saturé");
        let four = Some(4 * GIB);
        assert_eq!(g.best_fit(&st, four, at(402 + 300)), Some(2), "4B encore de côté");
        assert_eq!(g.best_fit(&st, four, at(402 + 361)), Some(1));
        // Réglages graphiques de WoW changés : plus rien de côté.
        g.graphics_changed();
        assert_eq!(g.best_fit(&st, four, at(410)), Some(1));
    }

    #[test]
    fn nothing_lighter_on_the_card_goes_to_the_processor_or_stops() {
        let mut st = steps();
        let mut g = Governor::new(8 * GIB);
        let t0 = Instant::now();
        // 1.7B sur la carte qui sature : le processeur.
        g.sample(&st, Some(2), 0, false, t0);
        assert_eq!(g.sample(&st, Some(2), 0, false, t0), Move::Down(Some(3)));
        // Sur le processeur : jamais de saturation imputée au modèle.
        assert_eq!(g.sample(&st, Some(3), 0, false, t0), Move::Stay);
        assert_eq!(g.sample(&st, Some(3), 0, false, t0), Move::Stay);
        // Sans étape sur le processeur : plus de modèle (le jeu passe d'abord).
        st.pop();
        let mut g = Governor::new(8 * GIB);
        g.sample(&st, Some(2), 0, false, t0);
        assert_eq!(g.sample(&st, Some(2), 0, false, t0), Move::Down(None));
        // Étapes absentes du PC : sautées.
        let mut st = steps();
        st[1].usable = false;
        assert_eq!(Governor::new(16 * GIB).best_fit(&st, Some(4 * GIB), t0), Some(2));
        assert_eq!(Governor::new(16 * GIB).best_fit(&st, None, t0), Some(0));
    }
}
