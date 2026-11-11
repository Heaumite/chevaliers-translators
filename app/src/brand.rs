//! Logo de Chevaliers Translators (médaillon d'or aux deux lions de gueules, anneau de fer) pour
//! ce que l'application dessine elle-même : icône de la fenêtre des modèles, médaillon du panneau
//! affiché sur le jeu, icône près de l'horloge à défaut de celle de l'exécutable. Source : le logo
//! 64x64 de l'addon (`Media/Logo.tga`), tiré de `media/logo.tga`.

use crate::res_icone::{decode_tga, shrink, Rgba};

const LOGO_64: &[u8] = include_bytes!("../../addon/ChevaliersTranslators/Media/Logo.tga");

/// Logo RGBA (alpha non prémultiplié), `size` x `size`.
pub fn logo(size: usize) -> Rgba {
    let l = decode_tga(LOGO_64).expect("Media/Logo.tga illisible");
    if size == l.w {
        l
    } else {
        shrink(&l, size.max(1))
    }
}

/// Le même en BGRA (bitmaps de Windows).
pub fn icon_bgra(size: usize) -> Vec<u8> {
    let mut px = logo(size).px;
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_at_icon_sizes() {
        for size in [16, 20, 24, 32, 48, 64] {
            let l = logo(size);
            assert_eq!((l.w, l.h, l.px.len()), (size, size, size * size * 4));
            let at = |x: usize, y: usize| &l.px[(y * size + x) * 4..][..4];
            // Coins transparents, médaillon opaque au centre, champ d'or en haut du médaillon.
            assert_eq!(at(0, 0)[3], 0);
            assert!(at(size / 2, size / 2)[3] >= 250);
            let field = at(size / 2, size / 4);
            assert!(field[0] > 150 && field[1] > 120 && field[2] < 90, "{size} : {field:?}");
        }
        let bgra = icon_bgra(32);
        let rgba = logo(32).px;
        assert_eq!((bgra[0..4].to_vec(), bgra[2]), (vec![rgba[2], rgba[1], rgba[0], rgba[3]], rgba[0]));
    }
}
