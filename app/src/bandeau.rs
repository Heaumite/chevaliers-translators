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

/// Zone de capture resserrée sur un bandeau trouvé (largeur, hauteur) : le bandeau, deux blocs de
/// marge à gauche, et quelques rangées au-dessus pour le cache.
pub fn narrow_size(g: &Geometry) -> (usize, usize) {
    (g.cols * g.block + 2 * g.block + 8, 2 * g.block + 6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode;
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
