//! Bandeau caché (0.15) : l'application pose sur le bandeau une image faite des pixels juste
//! au-dessus de lui (vue par le joueur, exclue des captures d'écran), et resserre sa capture sur
//! lui. Calculs communs, testés hors Windows.

use crate::decode::{Geometry, Image};
use crate::mirror::Canvas;

/// Pixels juste au-dessus du bandeau, pour le cacher.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverStrip {
    /// Coin haut-gauche du bandeau compté depuis le coin bas-droit de la zone de jeu (pixels).
    pub from_right: i32,
    pub from_bottom: i32,
    pub h: usize,
    /// Couleur de chaque colonne (rangée au-dessus du bandeau).
    pub row: Vec<[u8; 3]>,
}

/// Rangée au-dessus du bandeau trouvé (deux pixels plus haut, loin de son bord).
pub fn cover_strip(img: &Image, g: &Geometry) -> Option<CoverStrip> {
    let width = g.cols * g.block;
    let x0 = (g.right + 1).checked_sub(width)?;
    let y_top = (g.bottom + 1).checked_sub(2 * g.block)?;
    let src = y_top.checked_sub(2)?;
    let pad = 2usize;
    let xa = x0.saturating_sub(pad);
    let xb = (g.right + pad).min(img.w - 1);
    let row: Vec<[u8; 3]> = (xa..=xb)
        .map(|x| {
            let i = src * img.stride + x * 4;
            [img.bgra[i + 2], img.bgra[i + 1], img.bgra[i]]
        })
        .collect();
    Some(CoverStrip {
        from_right: (img.w - xa) as i32,
        from_bottom: (img.h - y_top) as i32,
        h: g.bottom + 1 - y_top,
        row,
    })
}

/// Image du cache : chaque colonne prend la couleur du pixel au-dessus, adoucie sur 5 colonnes.
pub fn cover_canvas(s: &CoverStrip) -> Canvas {
    if s.row.is_empty() {
        return Canvas::new(1, 1);
    }
    let w = s.row.len();
    let mut cv = Canvas::new(w, s.h.max(1));
    for x in 0..w {
        let (a, b) = (x.saturating_sub(2), (x + 2).min(w - 1));
        let n = (b - a + 1) as u32;
        let mut sum = [0u32; 3];
        for c in &s.row[a..=b] {
            for k in 0..3 {
                sum[k] += c[k] as u32;
            }
        }
        let c = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
        for y in 0..s.h {
            cv.blend(x as i32, y as i32, c, 1.0);
        }
    }
    cv
}

/// Garde du cache du bandeau (0.16.3). Le cache est retiré quand le bandeau n'est plus lu ; s'il
/// l'est juste après sa pose plusieurs fois de suite, la capture ne le traverse peut-être pas
/// (Windows qui n'exclut pas la fenêtre) : pause, puis nouvel essai. Avant, trois retraits en une
/// minute l'abandonnaient pour toute la session, et un changement de fenêtre suffisait.
/// - seuls comptent les retraits jeu au premier plan, rien devant lui ;
/// - une fois le bandeau lu à travers le cache (« prouvé »), plus d'abandon : pauses courtes ;
/// - sans preuve, trois pauses : abandon pour la session (vraie incompatibilité).
#[derive(Debug, Default)]
pub struct CoverGuard {
    proven: bool,
    strikes: Vec<std::time::Instant>,
    paused_until: Option<std::time::Instant>,
    pauses: u32,
    off: bool,
}

/// Retrait « rapide » : moins de ce temps après la pose.
pub const COVER_QUICK: std::time::Duration = std::time::Duration::from_millis(2500);
/// Bandeau lu à travers le cache posé depuis ce temps : le cache fonctionne.
pub const COVER_PROOF: std::time::Duration = std::time::Duration::from_millis(2000);

/// Ce que la garde décide après un retrait.
#[derive(Debug, PartialEq)]
pub enum CoverVerdict {
    Keep,
    Pause(std::time::Duration),
    Off,
}

impl CoverGuard {
    pub fn proven(&self) -> bool {
        self.proven
    }

    /// Cache autorisé maintenant (ni en pause, ni abandonné).
    pub fn allowed(&mut self, now: std::time::Instant) -> bool {
        if self.off {
            return false;
        }
        match self.paused_until {
            Some(t) if now < t => false,
            Some(_) => {
                self.paused_until = None;
                true
            }
            None => true,
        }
    }

    /// Image du bandeau reçue ; `shown_at` : cache posé sans interruption depuis.
    pub fn on_strip(&mut self, at: std::time::Instant, shown_at: Option<std::time::Instant>) {
        if shown_at.is_some_and(|s| at.saturating_duration_since(s) >= COVER_PROOF) {
            self.proven = true;
        }
    }

    /// Cache retiré faute de bandeau lu. `steady` : jeu au premier plan et rien devant lui.
    pub fn on_lost(&mut self, now: std::time::Instant, shown_at: std::time::Instant, steady: bool) -> CoverVerdict {
        if !steady || now.saturating_duration_since(shown_at) >= COVER_QUICK {
            return CoverVerdict::Keep;
        }
        let window = std::time::Duration::from_secs(if self.proven { 30 } else { 60 });
        self.strikes.push(now);
        self.strikes.retain(|t| now.saturating_duration_since(*t) < window);
        let limit = if self.proven { 6 } else { 3 };
        if self.strikes.len() < limit {
            return CoverVerdict::Keep;
        }
        self.strikes.clear();
        self.pauses += 1;
        if !self.proven && self.pauses >= 3 {
            self.off = true;
            return CoverVerdict::Off;
        }
        let d = std::time::Duration::from_secs(if self.proven { 30 } else { 60 });
        self.paused_until = Some(now + d);
        CoverVerdict::Pause(d)
    }
}

/// Cache posé en (gauche, haut) sur le bureau, en partie sous une autre fenêtre (gauche, haut,
/// droite, bas) : ses pixels sous elle deviennent transparents (rien n'est dessiné sur une autre
/// application). `None` : entièrement dessous.
pub fn clip_cover(cv: &Canvas, left: i32, top: i32, over: Option<(i32, i32, i32, i32)>) -> Option<Canvas> {
    let (w, h) = (cv.w as i32, cv.h as i32);
    let Some((ol, ot, or, ob)) = over else { return Some(cv.clone()) };
    let x0 = (ol - left).clamp(0, w);
    let x1 = (or - left).clamp(0, w);
    let y0 = (ot - top).clamp(0, h);
    let y1 = (ob - top).clamp(0, h);
    if x1 <= x0 || y1 <= y0 {
        return Some(cv.clone());
    }
    if x0 == 0 && y0 == 0 && x1 == w && y1 == h {
        return None;
    }
    let mut out = cv.clone();
    for y in y0..y1 {
        for x in x0..x1 {
            out.px[(y * w + x) as usize] = 0;
        }
    }
    Some(out)
}

/// Zone de capture resserrée sur un bandeau trouvé (largeur, hauteur) : le bandeau, deux blocs de
/// marge à gauche, et quelques rangées au-dessus pour le cache.
pub fn narrow_size(g: &Geometry) -> (usize, usize) {
    (g.cols * g.block + 2 * g.block + 8, 2 * g.block + 6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode;

    #[test]
    fn cover_guard_survives_window_switches() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // Changements de fenêtre (jeu pas devant, ou une fenêtre devant lui) : jamais compté.
        let mut g = CoverGuard::default();
        for i in 0..20 {
            assert_eq!(g.on_lost(ms(i * 1000 + 500), ms(i * 1000), false), CoverVerdict::Keep);
        }
        assert!(g.allowed(ms(30_000)));
        // Retraits lents (bandeau caché par le jeu longtemps après la pose) : pas comptés.
        for i in 0..10 {
            assert_eq!(g.on_lost(ms(40_000 + i * 5000 + 3000), ms(40_000 + i * 5000), true), CoverVerdict::Keep);
        }
        // Bandeau lu 2 s après la pose : le cache fonctionne ; ensuite, pauses courtes, jamais
        // d'abandon.
        g.on_strip(ms(102_100), Some(ms(100_000)));
        assert!(g.proven());
        let mut pauses = 0;
        let mut t = 110_000;
        for _ in 0..40 {
            if g.allowed(ms(t)) {
                for v in [g.on_lost(ms(t + 500), ms(t), true), g.on_lost(ms(t + 600), ms(t + 550), true)] {
                    assert_ne!(v, CoverVerdict::Off);
                    if let CoverVerdict::Pause(d) = v {
                        assert_eq!(d, Duration::from_secs(30));
                        pauses += 1;
                    }
                }
            }
            t += 2000;
        }
        assert!(pauses >= 2, "pauses : {pauses}");
        assert!(g.allowed(ms(t + 31_000)), "cache repris après la pause");
    }

    #[test]
    fn cover_guard_gives_up_only_when_never_proven() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut g = CoverGuard::default();
        let mut t = 0;
        let mut verdicts = Vec::new();
        while verdicts.len() < 3 && t < 1_000_000 {
            if g.allowed(ms(t)) {
                match g.on_lost(ms(t + 400), ms(t), true) {
                    CoverVerdict::Keep => {}
                    v => verdicts.push(v),
                }
            }
            t += 1000;
        }
        assert_eq!(verdicts, vec![CoverVerdict::Pause(Duration::from_secs(60)), CoverVerdict::Pause(Duration::from_secs(60)), CoverVerdict::Off]);
        assert!(!g.allowed(ms(t + 3_600_000)));
    }

    #[test]
    fn cover_clipped_under_another_window() {
        let mut cv = Canvas::new(10, 4);
        for p in cv.px.iter_mut() {
            *p = 0xff10_2030;
        }
        // Rien devant, ou à côté : intact.
        assert_eq!(clip_cover(&cv, 100, 50, None).unwrap().px, cv.px);
        assert_eq!(clip_cover(&cv, 100, 50, Some((0, 0, 100, 50))).unwrap().px, cv.px);
        // Fenêtre sur la moitié gauche : ces pixels transparents, le reste gardé.
        let c = clip_cover(&cv, 100, 50, Some((0, 0, 105, 200))).unwrap();
        assert!((0..4).all(|y| (0..5).all(|x| c.px[y * 10 + x] == 0) && (5..10).all(|x| c.px[y * 10 + x] == 0xff10_2030)));
        // Entièrement dessous : pas de cache.
        assert!(clip_cover(&cv, 100, 50, Some((90, 40, 120, 60))).is_none());
    }
    use crate::proto;
    use crate::render::{render, Distortion, Rng};

    #[test]
    fn narrowed_capture_still_reads_the_banner_and_the_cover_matches_what_is_above_it() {
        let mut rng = Rng::new(7);
        for (sw, block) in [(1920usize, 4usize), (3840, 4), (2560, 6)] {
            let cols = (sw as f32 * 0.25 / block as f32) as usize;
            let maxp = proto::capacity(cols).2;
            let payload = proto::test_payload(42, maxp);
            let syms = proto::build_banner(proto::FRAME_TEST, 42, &payload, cols).unwrap();
            // Zone large (30 % de l'écran, 48 rangées) comme au démarrage.
            let (w, h) = ((sw as f32 * 0.30) as usize + 8, 48);
            let img = render(&syms, cols, block, w, h, &Distortion::NONE, &mut rng);
            let full = Image { w, h, stride: w * 4, bgra: &img };
            let d = decode(&full).expect("bandeau lu dans la zone large");
            // Cache : la rangée deux pixels au-dessus du bandeau, sur toute sa largeur.
            let s = cover_strip(&full, &d.geometry).expect("rangée au-dessus du bandeau");
            assert_eq!(s.h, 2 * block);
            assert_eq!(s.from_bottom as usize, 2 * block);
            // Deux colonnes de plus à gauche (à droite, le bord de la zone de jeu).
            assert_eq!(s.row.len(), cols * block + 2);
            let y = h - 2 * block - 2;
            let x0 = w - s.from_right as usize;
            for (k, c) in s.row.iter().enumerate() {
                let i = (y * w + x0 + k) * 4;
                assert_eq!(*c, [img[i + 2], img[i + 1], img[i]]);
            }
            let cv = cover_canvas(&s);
            assert_eq!((cv.w, cv.h), (s.row.len(), 2 * block));
            assert!(cv.px.iter().all(|p| p >> 24 == 255), "cache opaque");
            // Zone resserrée (coin bas-droit) : le même bandeau, environ 4 fois moins de pixels.
            let (nw, nh) = narrow_size(&d.geometry);
            assert!(nw * nh * 3 < w * h, "{nw}x{nh} contre {w}x{h}");
            let mut crop = vec![0u8; nw * nh * 4];
            for yy in 0..nh {
                let src = ((h - nh + yy) * w + (w - nw)) * 4;
                crop[yy * nw * 4..(yy + 1) * nw * 4].copy_from_slice(&img[src..src + nw * 4]);
            }
            let small = Image { w: nw, h: nh, stride: nw * 4, bgra: &crop };
            let d2 = decode(&small).expect("bandeau lu dans la zone resserrée");
            assert_eq!(d2.frame, d.frame);
            let s2 = cover_strip(&small, &d2.geometry).expect("cache dans la zone resserrée");
            assert_eq!(s2, s, "même cache, même place depuis le coin bas-droit");
        }
    }
}
