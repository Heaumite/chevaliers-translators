//! Tests de l'icône de l'exécutable (fichier .res écrit par build.rs).

use crate::res_icone::*;

const LOGO: &[u8] = include_bytes!("../../media/logo.tga");

#[test]
fn logo_is_decoded_upright() {
    let logo = decode_tga(LOGO).unwrap();
    assert_eq!((logo.w, logo.h), (512, 512));
    let at = |x: usize, y: usize| &logo.px[(y * 512 + x) * 4..][..4];
    // Coins transparents, centre opaque et jaune (champ du médaillon).
    assert_eq!(at(0, 0)[3], 0);
    assert_eq!(at(511, 511)[3], 0);
    let c = at(256, 256);
    assert!(c[3] >= 250 && c[0] > 180 && c[1] > 150 && c[2] < 120, "champ jaune au centre : {c:?}");
}

#[test]
fn rle_and_top_first_tga() {
    // 2x2 : rouge, vert / bleu, blanc ; écrit ligne du haut d'abord, compressé.
    let mut b = vec![0u8, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
    b.extend_from_slice(&[0x81, 0, 0, 255, 255]); // 2 x rouge (BGRA)
    b.extend_from_slice(&[0x01, 255, 0, 0, 255, 255, 255, 255, 128]); // bleu, blanc semi-transparent
    let img = decode_tga(&b).unwrap();
    assert_eq!(&img.px[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
    assert_eq!(&img.px[8..], &[0, 0, 255, 255, 255, 255, 255, 128]);
}

#[test]
fn res_file_structure() {
    let res = logo_res(LOGO).unwrap();
    // Entrées : en-tête vide, une image par taille, le groupe.
    let mut pos = 0;
    let mut entries = Vec::new();
    while pos < res.len() {
        let u32_at = |p: usize| u32::from_le_bytes(res[p..p + 4].try_into().unwrap());
        let (size, head) = (u32_at(pos) as usize, u32_at(pos + 4) as usize);
        assert_eq!(head, 32);
        let kind = u16::from_le_bytes([res[pos + 10], res[pos + 11]]);
        let name = u16::from_le_bytes([res[pos + 14], res[pos + 15]]);
        entries.push((kind, name, pos + head, size));
        pos += (head + size).div_ceil(4) * 4;
    }
    assert_eq!(pos, res.len());
    assert_eq!(entries.len(), 2 + ICON_SIZES.len());
    assert_eq!((entries[0].0, entries[0].3), (0, 0));
    let (kind, name, at, size) = *entries.last().unwrap();
    assert_eq!((kind, name, size), (14, 1, 6 + 14 * ICON_SIZES.len()));
    for (i, s) in ICON_SIZES.iter().enumerate() {
        let (kind, name, data, size) = entries[i + 1];
        assert_eq!((kind, name as usize), (3, i + 1));
        // DIB : largeur, hauteur doublée, 32 bits ; taille annoncée dans le groupe.
        let w = i32::from_le_bytes(res[data + 4..data + 8].try_into().unwrap());
        let h = i32::from_le_bytes(res[data + 8..data + 12].try_into().unwrap());
        assert_eq!((w as usize, h as usize), (*s, s * 2));
        let g = at + 6 + 14 * i;
        assert_eq!(res[g] as usize, if *s == 256 { 0 } else { *s });
        assert_eq!(u32::from_le_bytes(res[g + 8..g + 12].try_into().unwrap()) as usize, size);
        assert_eq!(u16::from_le_bytes([res[g + 12], res[g + 13]]) as usize, i + 1);
    }
}

#[test]
fn small_icon_keeps_the_medallion() {
    let logo = decode_tga(LOGO).unwrap();
    let s = shrink(&logo, 16);
    let at = |x: usize, y: usize| &s.px[(y * 16 + x) * 4..][..4];
    assert_eq!(at(0, 0)[3], 0, "coin transparent");
    assert!(at(8, 8)[3] >= 250, "centre opaque");
    // Bord du disque partiellement couvert (alpha intermédiaire), couleur non assombrie.
    let edge = at(0, 8);
    assert!(edge[3] > 100 && edge[3] < 240, "bord adouci : {edge:?}");
    let field = at(8, 3);
    assert!(field[0] > 150 && field[2] < 100, "champ jaune : {field:?}");
}

#[test]
fn version_info_matches_llvm_rc() {
    // Référence : llvm-rc 18 sur un .rc VERSIONINFO équivalent (LANGUAGE 0, 0).
    let reference = include_bytes!("../tests/version_llvm_rc.res");
    let strings = [
        ("CompanyName", "Chevaliers"),
        ("FileDescription", "Chevaliers Translators"),
        ("FileVersion", "0.14.2"),
        ("ProductVersion", "0.14.2"),
    ];
    let res = version_res(&VersionInfo { version: [0, 14, 2, 0], strings: &strings });
    assert_eq!(res.as_slice(), reference.as_slice());
    // Icône et version ensemble : l'entrée de version en dernier, éditeur Chevaliers.
    let all = project_strings("0.14.2", "Chevaliers Translators", "ChevaliersTranslators", "ChevaliersTranslators.exe");
    let res = app_res(LOGO, &VersionInfo { version: [0, 14, 2, 0], strings: &all }).unwrap();
    let text: Vec<u16> = res.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let has = |s: &str| {
        let w: Vec<u16> = s.encode_utf16().collect();
        text.windows(w.len()).any(|x| x == w.as_slice()) || res[1..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>().windows(w.len()).any(|x| x == w.as_slice())
    };
    assert!(has("CompanyName") && has("Chevaliers") && has("ChevaliersTranslators.exe"));
}
