//! CT-Pixel v1 : format des trames, identique à `Encoder.lua`.

pub const PROTO_VERSION: u8 = 1;
pub const FRAME_TEST: u8 = 1;
pub const FRAME_CHAT: u8 = 2;
pub const FRAME_PING: u8 = 3;
pub const FRAME_SEND: u8 = 4;
pub const FRAME_RETARGET: u8 = 5;
pub const FRAME_MVIEW: u8 = 6;
pub const FRAME_MLINE: u8 = 7;
pub const FRAME_MADV: u8 = 8;
pub const FRAME_PERF: u8 = 9;
/// Demande de l'addon à l'application (texte : « essai 4b », « essai stop »...).
pub const FRAME_CMD: u8 = 10;
/// Signe de vie de l'addon tant que le chat miroir est actif (absent : déconnexion, écran de
/// chargement, choix du personnage).
pub const FRAME_ALIVE: u8 = 11;
/// État de l'envoi traduit (0.11) : [nature, numéro de demande sur 2 octets] ; nature 1 : boîte de
/// saisie rouverte, vide, prête pour le collage ; 2 : annulé ; 3 : parti.
pub const FRAME_STATE: u8 = 12;
/// Rectangles posés sur le chat (bulles d'aide, menus, fenêtres du jeu), 0.11 : [nombre, puis
/// gauche, bas, largeur, hauteur sur 2 octets chacun], pixels physiques depuis le bas à gauche.
pub const FRAME_OCCLUDE: u8 = 13;
/// Ligne de diagnostic de l'addon (/chevaliers diag), recopiée dans la console.
pub const FRAME_NOTE: u8 = 14;
/// Icône citée par une ligne du chat (0.12.3) : atlas (fichier et coordonnées) ou chemin de
/// texture et son FileDataID, tels que le jeu les connaît.
pub const FRAME_ICON: u8 = 15;
/// Zone (0.13.3) : carte du jeu (u16) puis numéros des canaux géographiques (Général...).
pub const FRAME_ZONE: u8 = 16;
/// Message reçu pendant le verrouillage du chat (combat en instance), relayé à sa fin (0.13.3).
pub const FRAME_LATE: u8 = 17;
/// Bulles de dialogue au-dessus des joueurs (0.15) : place, couleur et police de chacune, et le
/// numéro du message relayé qu'elle affiche (`mirror::bulles::parse`).
pub const FRAME_BUBBLE: u8 = 18;
pub const STATE_READY: u8 = 1;
pub const STATE_CANCEL: u8 = 2;
pub const STATE_SENT: u8 = 3;

pub const SYNC: [u8; 4] = [51, 12, 51, 12];
pub const CALIB: [u8; 4] = [0, 21, 42, 63];
pub const FIXED_BLOCKS: usize = 10;
pub const HEADER_BYTES: usize = 5;
pub const CRC_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub ftype: u8,
    pub seq: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodeError {
    NoSync,
    BadCalibration,
    BadGeometry,
    BadVersion,
    BadLength,
    Crc,
}

impl DecodeError {
    pub fn label(self) -> &'static str {
        match self {
            DecodeError::NoSync => "pas de bandeau",
            DecodeError::BadCalibration => "étalonnage illisible",
            DecodeError::BadGeometry => "géométrie hors zone capturée",
            DecodeError::BadVersion => "version inconnue",
            DecodeError::BadLength => "longueur invalide",
            DecodeError::Crc => "CRC faux",
        }
    }
}

/// (symboles de données, octets, charge utile maximale)
pub fn capacity(cols: usize) -> (usize, usize, usize) {
    let symbols = (2 * cols).saturating_sub(FIXED_BLOCKS);
    let bytes = symbols * 6 / 8;
    let payload = bytes.saturating_sub(HEADER_BYTES + CRC_BYTES).min(255);
    (symbols, bytes, payload)
}

fn crc_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    })
}

pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = (crc >> 8) ^ t[((crc ^ b as u32) & 0xFF) as usize];
    }
    crc ^ 0xFFFF_FFFF
}

/// Charge utile de test déterministe (même calcul que `CT.TestPayload`).
pub fn test_payload(seq: u16, max_len: usize) -> Vec<u8> {
    const M: u64 = 1 << 32;
    let len = (seq as usize * 37) % (max_len + 1);
    let mut s: u64 = (seq as u64 * 2_654_435_761 + 2_654_435_769) % M;
    (0..len)
        .map(|_| {
            s = (s * 1_664_525 + 1_013_904_223) % M;
            (s >> 24) as u8
        })
        .collect()
}

/// Construit les 2*cols symboles d'une trame (même ordre que le bandeau Lua).
pub fn build_banner(ftype: u8, seq: u16, payload: &[u8], cols: usize) -> Result<Vec<u8>, String> {
    let (nsym, _, maxp) = capacity(cols);
    if payload.len() > maxp {
        return Err(format!("charge utile {} > {}", payload.len(), maxp));
    }
    let mut b = Vec::with_capacity(HEADER_BYTES + payload.len() + CRC_BYTES);
    b.push(PROTO_VERSION);
    b.push(ftype);
    b.extend_from_slice(&seq.to_be_bytes());
    b.push(payload.len() as u8);
    b.extend_from_slice(payload);
    let crc = crc32(&b);
    b.extend_from_slice(&crc.to_be_bytes());

    let mut syms = Vec::with_capacity(2 * cols);
    syms.extend_from_slice(&SYNC);
    syms.extend_from_slice(&CALIB);
    syms.push(((cols / 64) % 64) as u8);
    syms.push((cols % 64) as u8);
    syms.extend(bytes_to_symbols(&b));
    syms.resize(FIXED_BLOCKS + nsym, 0);
    Ok(syms)
}

pub fn bytes_to_symbols(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 8 / 6 + 1);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(((acc >> bits) & 0x3F) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(((acc << (6 - bits)) & 0x3F) as u8);
    }
    out
}

pub fn symbols_to_bytes(syms: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(syms.len() * 6 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &s in syms {
        acc = (acc << 6) | (s & 0x3F) as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// Vérifie l'en-tête et le CRC d'une trame décodée.
pub fn parse_frame(bytes: &[u8]) -> Result<Frame, DecodeError> {
    if bytes.len() < HEADER_BYTES + CRC_BYTES {
        return Err(DecodeError::BadLength);
    }
    if bytes[0] != PROTO_VERSION {
        return Err(DecodeError::BadVersion);
    }
    let len = bytes[4] as usize;
    let end = HEADER_BYTES + len;
    if end + CRC_BYTES > bytes.len() {
        return Err(DecodeError::BadLength);
    }
    let want = u32::from_be_bytes([bytes[end], bytes[end + 1], bytes[end + 2], bytes[end + 3]]);
    if crc32(&bytes[..end]) != want {
        return Err(DecodeError::Crc);
    }
    Ok(Frame {
        ftype: bytes[1],
        seq: u16::from_be_bytes([bytes[2], bytes[3]]),
        payload: bytes[HEADER_BYTES..end].to_vec(),
    })
}

/// Fragments de chat : identifiant (2), numéro, total, canal, puis données.
pub fn chat_fragments(
    msg_id: u16,
    chan: u8,
    sender: &str,
    text: &str,
    maxp: usize,
) -> Vec<Vec<u8>> {
    let mut body = sender.as_bytes().to_vec();
    body.push(0x1F);
    body.extend_from_slice(text.as_bytes());
    let per = maxp.saturating_sub(5).max(1);
    let count = body.len().div_ceil(per).clamp(1, 255);
    body.truncate(count * per);
    (0..count)
        .map(|i| {
            let mut p = vec![
                (msg_id >> 8) as u8,
                msg_id as u8,
                (i + 1) as u8,
                count as u8,
                chan,
            ];
            let s = i * per;
            p.extend_from_slice(&body[s..(s + per).min(body.len())]);
            p
        })
        .collect()
}

pub fn channel_name(code: u8) -> &'static str {
    match code {
        1 => "Dire",
        2 => "Crier",
        3 => "Chuchotement reçu",
        4 => "Chuchotement envoyé",
        5 => "Groupe",
        6 => "Chef de groupe",
        7 => "Raid",
        8 => "Chef de raid",
        9 => "Avertissement raid",
        10 => "Guilde",
        11 => "Officier",
        12 => "Instance",
        13 => "Chef d'instance",
        14 => "Canal",
        15 => "Emote",
        16 => "Battle.net reçu",
        17 => "Battle.net envoyé",
        18 => "Guilde Discord",
        19 => "Communauté",
        20 => "Transcription vocale",
        21 => "Absent",
        22 => "Occupé",
        23 => "Battle.net diffusion",
        65..=127 => "Canal",
        _ => "?",
    }
}

/// Code d'un canal numéroté (1. Général, 2. Commerce...) : 64 + numéro (addon 0.12).
pub const CHANNEL_NUMBERED: u8 = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_reference() {
        // Valeur de référence du CRC-32 IEEE.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn symbols_roundtrip() {
        let data: Vec<u8> = (0..=255u8).collect();
        let s = bytes_to_symbols(&data);
        assert_eq!(&symbols_to_bytes(&s)[..data.len()], &data[..]);
    }

    #[test]
    fn frame_roundtrip() {
        for cols in [40usize, 87, 120, 240] {
            let (_, nbytes, maxp) = capacity(cols);
            for seq in [0u16, 1, 37, 255, 1000, 65535] {
                let p = test_payload(seq, maxp);
                let syms = build_banner(FRAME_TEST, seq, &p, cols).unwrap();
                assert_eq!(syms.len(), 2 * cols);
                let bytes = symbols_to_bytes(&syms[FIXED_BLOCKS..]);
                let f = parse_frame(&bytes[..nbytes]).unwrap();
                assert_eq!(
                    f,
                    Frame {
                        ftype: FRAME_TEST,
                        seq,
                        payload: p
                    }
                );
            }
        }
    }

    /// Les vecteurs produits par l'addon Lua (Lua 5.1 et LuaJIT) doivent être identiques.
    #[test]
    fn lua_vectors_match() {
        let text = include_str!("../tests/lua_vectors.txt");
        let mut n = 0;
        for line in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let mut it = line.split(' ');
            let kind = it.next().unwrap();
            let cols: usize = it.next().unwrap().parse().unwrap();
            let seq: u16 = it.next().unwrap().parse().unwrap();
            let hex = it.next().unwrap();
            let lua: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let maxp = capacity(cols).2;
            let rust = match kind {
                "T" => build_banner(FRAME_TEST, seq, &test_payload(seq, maxp), cols).unwrap(),
                "C" => {
                    let part: usize = it.next().unwrap().parse().unwrap();
                    let frags = chat_fragments(seq, 10, "Hans-Ysondre", VECTOR_CHAT_TEXT, maxp);
                    build_banner(FRAME_CHAT, seq, &frags[part - 1], cols).unwrap()
                }
                _ => panic!("type inconnu {kind}"),
            };
            assert_eq!(lua, rust, "vecteur divergent : {line}");
            n += 1;
        }
        assert!(n > 20, "trop peu de vecteurs : {n}");
    }

    pub const VECTOR_CHAT_TEXT: &str = "Guten Abend zusammen ! Wer kommt mit in den Schwarzfels ? Ça fait plaisir, ŝ€ ✓ Привет всем |cffffd000|Hitem:19019::::::::60:::::|h[Donnerzorn]|h|r et encore un peu de texte pour dépasser la taille d'un fragment, encore et encore, jusqu'à la fin.";
}
