//! Chat miroir (S5) : l'addon décrit chaque fenêtre de chat (position, police, lignes
//! visibles et leur âge) ; on redessine ces lignes, avec la police extraite du jeu,
//! dans une image de la taille de la fenêtre, posée ensuite par-dessus le jeu.
//!
//! Mise en page identique à celle de WoW : messages empilés depuis le bas, chacun
//! coupé aux espaces à la largeur de la fenêtre (mot trop long coupé n'importe où),
//! pas de ligne = hauteur de ligne + interligne, effacement après `timeVisible` secondes
//! sur `fadeDuration` secondes, seulement quand la fenêtre est au bas de l'historique.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub const FLAG_VISIBLE: u8 = 1;
pub const FLAG_CALAGE: u8 = 2;
pub const FLAG_OFFICIAL: u8 = 4;
pub const FLAG_FADE: u8 = 8;
pub const FLAG_TRUNC: u8 = 16;
/// 0.12.3 : chaque ligne porte son nombre de lignes à l'écran (4 bits hauts de l'âge) ; une
/// ligne numéro 0 est dessinée par WoW (butin, système, créatures...) : seule sa place compte.
pub const FLAG_ROWS: u8 = 32;
/// 0.14, dispositif télévision : le texte du chat du jeu n'est pas caché ; le chat traduit le
/// recouvre (plaque sombre sous les lignes des joueurs).
pub const FLAG_COVER: u8 = 64;
/// Plaque du dispositif télévision : champ noir de la bannière, presque opaque.
pub const PLATE: Rgb = [14, 13, 10];
pub const PLATE_ALPHA: f32 = 0.94;

pub type Rgb = [u8; 3];

// --- Description d'une fenêtre ---------------------------------------------------

#[derive(Clone, Debug)]
pub struct View {
    pub frame: u8,
    pub flags: u8,
    /// Position et taille en pixels physiques, origine en bas à gauche de l'écran.
    pub left: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
    pub font_px: f32,
    pub line_h: f32,
    pub spacing: f32,
    pub shadow: (f32, f32, u8),
    pub font_flags: u8,
    pub time_visible: f32,
    pub fade: f32,
    pub font_path: String,
    /// (identifiant de ligne, âge en secondes à la réception), de bas en haut. Identifiant 0 :
    /// ligne dessinée par WoW.
    pub lines: Vec<(u16, f32)>,
    /// Nombre de lignes à l'écran de chacune dans WoW (0 : inconnu).
    pub rows: Vec<u8>,
    pub received: Instant,
    /// Pixels par unité de l'interface (taille des icônes) ; 0 : addon antérieur à la 0.12.3.
    pub ui_k: f32,
}

impl View {
    /// Vrai si l'application doit dessiner cette fenêtre.
    pub fn drawn(&self) -> bool {
        self.flags & FLAG_VISIBLE != 0 && self.flags & FLAG_OFFICIAL == 0 && self.width > 0
    }

    pub fn alpha(&self, age0: f32, now: Instant) -> f32 {
        if self.flags & FLAG_FADE == 0 {
            return 1.0;
        }
        let age = age0 + now.saturating_duration_since(self.received).as_secs_f32();
        if age <= self.time_visible {
            1.0
        } else if self.fade <= 0.0 || age - self.time_visible >= self.fade {
            0.0
        } else {
            1.0 - (age - self.time_visible) / self.fade
        }
    }
}

pub fn parse_view(p: &[u8], received: Instant) -> Result<View, String> {
    if p.len() < 2 {
        return Err("description trop courte".into());
    }
    let mut v = View {
        frame: p[0],
        flags: p[1],
        left: 0,
        bottom: 0,
        width: 0,
        height: 0,
        font_px: 14.0,
        line_h: 0.0,
        spacing: 0.0,
        shadow: (0.0, 0.0, 0),
        font_flags: 0,
        time_visible: 120.0,
        fade: 3.0,
        font_path: String::new(),
        lines: Vec::new(),
        rows: Vec::new(),
        received,
        ui_k: 0.0,
    };
    if v.flags & FLAG_VISIBLE == 0 {
        return Ok(v);
    }
    let u16_at = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]);
    if p.len() < 26 {
        return Err("description tronquée".into());
    }
    v.left = u16_at(2) as i32;
    v.bottom = u16_at(4) as i32;
    v.width = u16_at(6) as i32;
    v.height = u16_at(8) as i32;
    v.font_px = u16_at(10) as f32 / 10.0;
    v.line_h = u16_at(12) as f32 / 10.0;
    v.spacing = u16_at(14) as f32 / 10.0;
    v.shadow = (p[16] as i8 as f32 / 10.0, p[17] as i8 as f32 / 10.0, p[18]);
    v.font_flags = p[19];
    v.time_visible = u16_at(20) as f32 / 10.0;
    v.fade = u16_at(22) as f32 / 10.0;
    let plen = p[24] as usize;
    if p.len() < 26 + plen {
        return Err("description tronquée (police)".into());
    }
    v.font_path = String::from_utf8_lossy(&p[25..25 + plen]).into_owned();
    let n = p[25 + plen] as usize;
    let start = 26 + plen;
    if p.len() < start + 4 * n {
        return Err("description tronquée (lignes)".into());
    }
    let packed = v.flags & FLAG_ROWS != 0;
    v.lines = (0..n)
        .map(|j| {
            let o = start + 4 * j;
            let a = u16_at(o + 2);
            (u16_at(o), if packed { (a & 4095) as f32 / 10.0 } else { a as f32 / 10.0 })
        })
        .collect();
    v.rows = (0..n).map(|j| if packed { (u16_at(start + 4 * j + 2) >> 12) as u8 } else { 0 }).collect();
    let k = start + 4 * n;
    if p.len() >= k + 2 {
        v.ui_k = u16_at(k) as f32 / 1000.0;
    }
    Ok(v)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineText {
    pub color: Rgb,
    pub text: String,
}

/// Corps d'une trame de ligne : « RRGGBB\x1Ftexte ».
pub fn parse_line_body(body: &str) -> LineText {
    let (hex, text) = body.split_once('\u{1F}').unwrap_or(("FFFFFF", body));
    let c = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    LineText {
        color: [c(0).unwrap_or(255), c(2).unwrap_or(255), c(4).unwrap_or(255)],
        text: text.to_string(),
    }
}

// --- Balisage du chat WoW -----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Char(char, Rgb),
    /// Icône |T...|t ou |A...|a, à sa taille dans WoW (0.12.3 ; avant : un carré gris).
    Icon(crate::icons::IconRef),
    Break,
    /// Icône de copie ajoutée par le miroir à la fin d'un message de joueur (0.11).
    Copy,
}

fn named_color(name: &str) -> Option<Rgb> {
    Some(match name {
        // Qualité des objets (liens du retail : |cnIQ4:[Objet]|r).
        "IQ0" => [157, 157, 157],
        "IQ1" => [255, 255, 255],
        "IQ2" => [30, 255, 0],
        "IQ3" => [0, 112, 221],
        "IQ4" => [163, 53, 238],
        "IQ5" => [255, 128, 0],
        "IQ6" => [230, 204, 128],
        "IQ7" | "IQ8" => [0, 204, 255],
        "NORMAL_FONT_COLOR" => [255, 209, 0],
        "HIGHLIGHT_FONT_COLOR" | "WHITE_FONT_COLOR" => [255, 255, 255],
        "GREEN_FONT_COLOR" => [25, 255, 25],
        "RED_FONT_COLOR" => [255, 32, 32],
        "GRAY_FONT_COLOR" | "DISABLED_FONT_COLOR" => [128, 128, 128],
        "YELLOW_FONT_COLOR" => [255, 255, 0],
        "LIGHTBLUE_FONT_COLOR" => [133, 205, 255],
        "ORANGE_FONT_COLOR" => [255, 128, 64],
        "EPIC_PURPLE_COLOR" => [163, 53, 238],
        "RARE_BLUE_COLOR" => [0, 112, 221],
        "UNCOMMON_GREEN_COLOR" => [30, 255, 0],
        "LEGENDARY_ORANGE_COLOR" => [255, 128, 0],
        "ARTIFACT_GOLD_COLOR" => [230, 204, 128],
        "HEIRLOOM_BLUE_COLOR" => [0, 204, 255],
        "LINK_FONT_COLOR" => [102, 187, 255],
        "BLUE_FONT_COLOR" => [0, 112, 221],
        "WARNING_FONT_COLOR" => [255, 72, 0],
        "ACCOUNT_WIDE_FONT_COLOR" => [0, 204, 255],
        "DIM_RED_FONT_COLOR" => [204, 25, 25],
        "TRANSMOGRIFY_FONT_COLOR" => [255, 128, 255],
        "DARKYELLOW_FONT_COLOR" => [255, 209, 0],
        _ => {
            // Couleur de classe : |cnCLASS_<CLASSE>:... ou nom de classe seul.
            let c = name.strip_prefix("CLASS_").unwrap_or(name);
            match c {
                "DEATHKNIGHT" => [196, 30, 58],
                "DEMONHUNTER" => [163, 48, 201],
                "DRUID" => [255, 124, 10],
                "EVOKER" => [51, 147, 127],
                "HUNTER" => [170, 211, 114],
                "MAGE" => [63, 199, 235],
                "MONK" => [0, 255, 152],
                "PALADIN" => [244, 140, 186],
                "PRIEST" => [255, 255, 255],
                "ROGUE" => [255, 244, 104],
                "SHAMAN" => [0, 112, 221],
                "WARLOCK" => [135, 136, 238],
                "WARRIOR" => [198, 155, 109],
                _ => return None,
            }
        }
    })
}

/// Texte affiché d'une ligne de chat : couleurs |c...|r, liens |H...|h[texte]|h,
/// icônes |T|t et |A|a, noms protégés |K|k (illisibles hors du jeu), || et |n.
pub fn parse_markup(text: &str, base: Rgb) -> Vec<Item> {
    let ch: Vec<char> = text.chars().collect();
    let find = |from: usize, a: char, b: char| -> Option<usize> {
        (from..ch.len().saturating_sub(1)).find(|&j| ch[j] == a && ch[j + 1] == b)
    };
    let mut out = Vec::with_capacity(ch.len());
    let mut stack = vec![base];
    let mut i = 0;
    while i < ch.len() {
        let cur = *stack.last().unwrap();
        let c = ch[i];
        if c == '|' && i + 1 < ch.len() {
            match ch[i + 1] {
                '|' => {
                    out.push(Item::Char('|', cur));
                    i += 2;
                    continue;
                }
                'c' => {
                    if ch.get(i + 2) == Some(&'n') {
                        if let Some(colon) = (i + 3..ch.len()).find(|&j| ch[j] == ':') {
                            let name: String = ch[i + 3..colon].iter().collect();
                            stack.push(named_color(&name).unwrap_or(cur));
                            i = colon + 1;
                            continue;
                        }
                    } else if i + 10 <= ch.len() {
                        let hex: String = ch[i + 2..i + 10].iter().collect();
                        if let Ok(v) = u32::from_str_radix(&hex, 16) {
                            stack.push([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                            i += 10;
                            continue;
                        }
                    }
                }
                'r' => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    i += 2;
                    continue;
                }
                'H' => {
                    // Données du lien jusqu'au |h ; le texte affiché suit, fermé par un autre |h.
                    i = find(i + 2, '|', 'h').map_or(ch.len(), |j| j + 2);
                    continue;
                }
                'h' => {
                    i += 2;
                    continue;
                }
                'T' | 'A' => {
                    let atlas = ch[i + 1] == 'A';
                    let close = if atlas { 'a' } else { 't' };
                    let end = find(i + 2, '|', close);
                    let inner: String = ch[i + 2..end.unwrap_or(ch.len())].iter().collect();
                    out.push(Item::Icon(if atlas { crate::icons::parse_atlas(&inner) } else { crate::icons::parse_texture(&inner) }));
                    i = end.map_or(ch.len(), |j| j + 2);
                    continue;
                }
                'K' => {
                    for q in "???".chars() {
                        out.push(Item::Char(q, cur));
                    }
                    i = find(i + 2, '|', 'k').map_or(ch.len(), |j| j + 2);
                    continue;
                }
                'n' => {
                    out.push(Item::Break);
                    i += 2;
                    continue;
                }
                '4' => {
                    // Accord grammatical |4singulier:pluriel; : première forme.
                    if let Some(semi) = (i + 2..ch.len()).find(|&j| ch[j] == ';') {
                        for &q in ch[i + 2..semi].iter().take_while(|&&q| q != ':') {
                            out.push(Item::Char(q, cur));
                        }
                        i = semi + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if c == '\n' {
            out.push(Item::Break);
        } else if c != '\r' {
            out.push(Item::Char(c, cur));
        }
        i += 1;
    }
    out
}

/// Texte affiché d'une ligne, sans balisage (copie, journal).
pub fn plain(text: &str) -> String {
    parse_markup(text, [255, 255, 255])
        .iter()
        .filter_map(|it| match it {
            Item::Char(c, _) => Some(*c),
            Item::Break => Some(' '),
            _ => None,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// « [DE] Bonjour » -> « Bonjour » : l'acronyme de langue posé par la traduction.
pub fn strip_tag(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix('[') {
        if let Some((code, after)) = rest.split_once("] ") {
            if (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_uppercase()) {
                return after;
            }
        }
    }
    text
}

fn is_space(it: &Item) -> bool {
    matches!(it, Item::Char(' ', _))
}

/// Coupe en lignes de largeur `width` : (abscisse, élément) par ligne. `advs[i]` est
/// l'avance de `items[i]` (crénage avec le suivant compris) ; les lignes après la
/// première commencent à `indent` (retrait des lignes coupées du chat de WoW).
pub fn wrap(items: &[Item], advs: &[f32], width: f32, indent: f32) -> Vec<Vec<(f32, Item)>> {
    let mut rows: Vec<Vec<(f32, Item)>> = vec![Vec::new()];
    let mut x = 0.0f32;
    let trim = |row: &mut Vec<(f32, Item)>| {
        while row.last().is_some_and(|(_, it)| is_space(it)) {
            row.pop();
        }
    };
    let mut i = 0;
    while i < items.len() {
        let it = items[i];
        if it == Item::Break {
            trim(rows.last_mut().unwrap());
            rows.push(Vec::new());
            x = indent;
            i += 1;
            continue;
        }
        if is_space(&it) {
            if !rows.last().unwrap().is_empty() {
                rows.last_mut().unwrap().push((x, it));
                x += advs[i];
            }
            i += 1;
            continue;
        }
        let mut end = i;
        let mut w = 0.0;
        while end < items.len() && items[end] != Item::Break && !is_space(&items[end]) {
            w += advs[end];
            end += 1;
        }
        let has_content = rows.last().unwrap().iter().any(|(_, it)| !is_space(it));
        if x + w > width + 0.01 && has_content {
            trim(rows.last_mut().unwrap());
            rows.push(Vec::new());
            x = indent;
        }
        for k in i..end {
            // Mot plus long que la fenêtre : coupé n'importe où.
            if x + advs[k] > width + 0.01 && !rows.last().unwrap().is_empty() {
                rows.push(Vec::new());
                x = indent;
            }
            rows.last_mut().unwrap().push((x, items[k]));
            x += advs[k];
        }
        i = end;
    }
    trim(rows.last_mut().unwrap());
    rows
}

// --- Mesures de la police faites dans le jeu --------------------------------------

/// Avances des caractères, crénage et retrait, mesurés par l'addon avec la police du chat
/// à sa taille réelle : la mise en page reproduit alors exactement celle de WoW.
#[derive(Clone, Debug, Default)]
pub struct FontMetrics {
    pub adv: HashMap<char, f32>,
    /// Crénage mesuré dans le jeu pour les paires où il n'est pas nul. L'addon mesure
    /// toutes les paires des lignes affichées : une paire absente vaut 0 dans WoW.
    pub kern: HashMap<(char, char), f32>,
    pub indent: Option<f32>,
}

pub enum Measures {
    Adv(Vec<(char, f32)>),
    Kern(Vec<(char, char, f32)>),
    Indent(f32),
}

/// Trame de mesures : fenêtre, taille ×10, nature (0 avances, 1 crénage, 2 retrait), données.
pub fn parse_measures(p: &[u8]) -> Result<(u8, u16, Measures), String> {
    if p.len() < 4 {
        return Err("mesures trop courtes".into());
    }
    let size10 = u16::from_be_bytes([p[1], p[2]]);
    let cp = |i: usize| {
        char::from_u32(((p[i] as u32) << 16) | ((p[i + 1] as u32) << 8) | p[i + 2] as u32)
            .unwrap_or('\u{FFFD}')
    };
    let m = match p[3] {
        0 => {
            let n = *p.get(4).ok_or("mesures tronquées")? as usize;
            if p.len() < 5 + 5 * n {
                return Err("mesures tronquées".into());
            }
            Measures::Adv(
                (0..n)
                    .map(|j| {
                        let o = 5 + 5 * j;
                        (cp(o), u16::from_be_bytes([p[o + 3], p[o + 4]]) as f32 / 64.0)
                    })
                    .collect(),
            )
        }
        1 => {
            let n = *p.get(4).ok_or("mesures tronquées")? as usize;
            if p.len() < 5 + 8 * n {
                return Err("mesures tronquées".into());
            }
            Measures::Kern(
                (0..n)
                    .map(|j| {
                        let o = 5 + 8 * j;
                        (cp(o), cp(o + 3), i16::from_be_bytes([p[o + 6], p[o + 7]]) as f32 / 64.0)
                    })
                    .collect(),
            )
        }
        2 if p.len() >= 6 => Measures::Indent(u16::from_be_bytes([p[4], p[5]]) as f32 / 64.0),
        k => return Err(format!("mesures de nature {k} inconnue")),
    };
    Ok((p[0], size10, m))
}

// --- Image -----------------------------------------------------------------------

/// Image en BGRA prémultiplié, une valeur 0xAARRGGBB par pixel, lignes de haut en bas.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![0; w * h],
        }
    }

    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = y as usize * self.w + x as usize;
        let d = self.px[i];
        let inv = 1.0 - a;
        let ch = |shift: u32, src: f32| -> u32 {
            let dv = ((d >> shift) & 0xFF) as f32;
            ((src + dv * inv).round().min(255.0)) as u32
        };
        let na = ch(24, 255.0 * a);
        let nr = ch(16, c[0] as f32 * a);
        let ng = ch(8, c[1] as f32 * a);
        let nb = ch(0, c[2] as f32 * a);
        self.px[i] = (na << 24) | (nr << 16) | (ng << 8) | nb;
    }

    /// BMP 24 bits sur fond uni (pour voir le rendu hors du jeu).
    pub fn to_bmp(&self, bg: Rgb) -> Vec<u8> {
        let row = (self.w * 3).div_ceil(4) * 4;
        let size = 54 + row * self.h;
        let mut b = Vec::with_capacity(size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(self.w as i32).to_le_bytes());
        b.extend_from_slice(&(self.h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        for y in (0..self.h).rev() {
            let start = b.len();
            for x in 0..self.w {
                let p = self.px[y * self.w + x];
                let a = (p >> 24) as f32 / 255.0;
                let mix = |shift: u32, bgc: u8| -> u8 {
                    (((p >> shift) & 0xFF) as f32 + bgc as f32 * (1.0 - a)).round().min(255.0) as u8
                };
                b.push(mix(0, bg[2]));
                b.push(mix(8, bg[1]));
                b.push(mix(16, bg[0]));
            }
            b.resize(start + row, 0);
        }
        b
    }
}

// --- Polices ---------------------------------------------------------------------

pub struct FontBook {
    fonts: Vec<fontdue::Font>,
    names: Vec<String>,
    primary: HashMap<String, Option<usize>>,
    fallbacks: Vec<usize>,
    glyphs: HashMap<(usize, char, u32), (fontdue::Metrics, Vec<u8>)>,
}

pub fn font_key(path: &str) -> String {
    path.replace('/', "\\").to_ascii_uppercase()
}

impl Default for FontBook {
    fn default() -> Self {
        Self::new()
    }
}

impl FontBook {
    pub fn new() -> FontBook {
        FontBook {
            fonts: Vec::new(),
            names: Vec::new(),
            primary: HashMap::new(),
            fallbacks: Vec::new(),
            glyphs: HashMap::new(),
        }
    }

    fn add(&mut self, name: &str, data: Vec<u8>) -> Result<usize, String> {
        let f = fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
            .map_err(|e| format!("{name} : {e}"))?;
        self.fonts.push(f);
        self.names.push(name.to_string());
        Ok(self.fonts.len() - 1)
    }

    pub fn add_fallback(&mut self, name: &str, data: Vec<u8>) -> Result<(), String> {
        let i = self.add(name, data)?;
        self.fallbacks.push(i);
        Ok(())
    }

    /// Police du jeu pour un chemin (`None` mémorise un échec : on ne réessaie pas).
    pub fn set_primary(&mut self, path: &str, data: Option<Vec<u8>>) -> Result<(), String> {
        let idx = match data {
            Some(d) => Some(self.add(path, d)?),
            None => None,
        };
        self.primary.insert(font_key(path), idx);
        Ok(())
    }

    pub fn knows(&self, path: &str) -> bool {
        self.primary.contains_key(&font_key(path))
    }

    pub fn fallback_names(&self) -> Vec<&str> {
        self.fallbacks.iter().map(|i| self.names[*i].as_str()).collect()
    }

    fn base(&self, path: &str) -> Option<usize> {
        self.primary
            .get(&font_key(path))
            .copied()
            .flatten()
            .or_else(|| self.fallbacks.first().copied())
    }

    /// Police qui dessine `c` : celle du jeu, sinon la première police de secours qui l'a.
    fn pick(&self, base: Option<usize>, c: char) -> Option<usize> {
        let has = |i: usize| self.fonts[i].lookup_glyph_index(c) != 0;
        base.filter(|&b| has(b))
            .or_else(|| self.fallbacks.iter().copied().find(|&i| has(i)))
            .or(base)
    }

    fn glyph(&mut self, font: usize, c: char, px: f32) -> &(fontdue::Metrics, Vec<u8>) {
        let key = (font, c, (px * 8.0).round() as u32);
        let fonts = &self.fonts;
        self.glyphs
            .entry(key)
            .or_insert_with(|| fonts[font].rasterize(c, px))
    }

    /// Ascendante et hauteur de ligne naturelles de la police à `px`.
    fn line_metrics(&self, base: Option<usize>, px: f32) -> (f32, f32) {
        base.and_then(|b| self.fonts[b].horizontal_line_metrics(px))
            .map(|m| (m.ascent, m.ascent - m.descent))
            .unwrap_or((px * 0.8, px))
    }
}

// --- Rendu -------------------------------------------------------------------------

/// Règles de mise en page tirées des mesures du jeu (ou estimées sans elles).
pub struct Layout {
    /// Pixels par unité de l'interface (taille des icônes).
    pub k: f32,
    /// Avances de fontdue multipliées par ce rapport pour les caractères non mesurés.
    pub ratio: f32,
    /// Paires de crénage connues (mesurées dans le jeu).
    pub kern_pairs: usize,
    pub indent: f32,
}

impl Layout {
    pub fn new(book: &mut FontBook, base: Option<usize>, px: f32, line_h: f32, m: Option<&FontMetrics>) -> Layout {
        let mut ratios: Vec<f32> = Vec::new();
        if let Some(m) = m {
            for (c, a) in &m.adv {
                if let Some(f) = base.filter(|&b| book.fonts[b].lookup_glyph_index(*c) != 0) {
                    let fa = book.glyph(f, *c, px).0.advance_width;
                    if fa > 0.5 && *a > 0.0 {
                        ratios.push(a / fa);
                    }
                }
            }
        }
        ratios.sort_by(|a, b| a.total_cmp(b));
        Layout {
            // Sans l'échelle de l'interface : police du chat supposée en taille 14.
            k: px / 14.0,
            ratio: ratios.get(ratios.len() / 2).copied().unwrap_or(1.0),
            kern_pairs: m.map_or(0, |m| m.kern.len()),
            // Sans mesure : retrait d'une hauteur de ligne, constaté au calage (v0.4.0).
            indent: m.and_then(|m| m.indent).unwrap_or(line_h),
        }
    }
}

/// Avance de chaque élément : mesurée dans le jeu si possible, sinon celle de la police
/// corrigée par `ratio` ; plus le crénage mesuré dans le jeu avec l'élément suivant.
/// Caractère absent de la police du jeu (WoW dessine un carré) : avance de la police
/// de secours qui le dessine, pour qu'il ne soit pas suivi d'un blanc.
fn advances(book: &mut FontBook, base: Option<usize>, px: f32, items: &[Item], m: Option<&FontMetrics>, l: &Layout) -> Vec<f32> {
    let mut out = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let a = match it {
            Item::Char(c, _) => {
                let in_game_font = base.is_some_and(|b| book.fonts[b].lookup_glyph_index(*c) != 0);
                match (in_game_font, m.and_then(|m| m.adv.get(c))) {
                    (true, Some(a)) => *a,
                    (true, None) => {
                        book.glyph(base.unwrap(), *c, px).0.advance_width * l.ratio
                    }
                    (false, measured) => match book.pick(base, *c) {
                        Some(f) if Some(f) != base => book.glyph(f, *c, px).0.advance_width,
                        _ => measured.copied().unwrap_or(px * 0.5),
                    },
                }
            }
            Item::Icon(r) => crate::icons::size_px(r, px, l.k).0,
            Item::Copy => copy_size(px) as f32 + 2.0,
            Item::Break => 0.0,
        };
        let k = match (it, items.get(i + 1), m) {
            (Item::Char(a, _), Some(Item::Char(b, _)), Some(m)) => m.kern.get(&(*a, *b)).copied().unwrap_or(0.0),
            _ => 0.0,
        };
        out.push(a + k);
    }
    out
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOpts {
    /// Décalages de calage en pixels (positifs : vers la droite, vers le bas).
    pub dx: f32,
    pub dy: f32,
    /// Dispositif télévision : plaque (couleur, opacité) sous les lignes des joueurs.
    pub plate: Option<(Rgb, f32)>,
}

const MAGENTA: Rgb = [255, 0, 255];
const BLACK: Rgb = [0, 0, 0];

/// Icône de copie à cliquer, en pixels de l'image (origine en haut à gauche).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub line: u16,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Hit {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Côté de l'icône de copie pour une police de `px` pixels.
fn copy_size(px: f32) -> i32 {
    (px * 0.8).round().max(8.0) as i32
}

/// Trait lissé de (x0, y0) à (x1, y1).
fn stroke(cv: &mut Canvas, (x0, y0): (f32, f32), (x1, y1): (f32, f32), width: f32, color: Rgb, a: f32) {
    let r = width / 2.0 + 1.0;
    let (minx, maxx) = (x0.min(x1) - r, x0.max(x1) + r);
    let (miny, maxy) = (y0.min(y1) - r, y0.max(y1) + r);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for py in miny.floor() as i32..=maxy.ceil() as i32 {
        for px in minx.floor() as i32..=maxx.ceil() as i32 {
            let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
            let t = (((cx - x0) * dx + (cy - y0) * dy) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (x0 + t * dx - cx, y0 + t * dy - cy);
            let cov = (width / 2.0 + 0.5 - (qx * qx + qy * qy).sqrt()).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(px, py, color, a * cov);
            }
        }
    }
}

/// Icône de copie (deux feuilles l'une sur l'autre), ou coche verte juste après un clic.
fn draw_copy(cv: &mut Canvas, x0: i32, y0: i32, s: i32, copied: bool, calage: bool, alpha: f32) {
    let (x, y, s) = (x0 as f32, y0 as f32, s as f32);
    let w = (s / 9.0).max(1.2);
    if copied {
        let c = if calage { MAGENTA } else { [90, 230, 90] };
        stroke(cv, (x + 0.12 * s, y + 0.55 * s), (x + 0.4 * s, y + 0.82 * s), w * 1.4, c, alpha);
        stroke(cv, (x + 0.4 * s, y + 0.82 * s), (x + 0.9 * s, y + 0.18 * s), w * 1.4, c, alpha);
        return;
    }
    let c = if calage { MAGENTA } else { [215, 215, 215] };
    let a = 0.85 * alpha;
    let side = s * 0.66;
    let rect = |cv: &mut Canvas, rx: f32, ry: f32| {
        stroke(cv, (rx, ry), (rx + side, ry), w, c, a);
        stroke(cv, (rx + side, ry), (rx + side, ry + side), w, c, a);
        stroke(cv, (rx + side, ry + side), (rx, ry + side), w, c, a);
        stroke(cv, (rx, ry + side), (rx, ry), w, c, a);
    };
    rect(cv, x + 0.06 * s, y + 0.06 * s);
    // Feuille de devant : fond sombre puis contour.
    let (fx, fy) = (x + 0.3 * s, y + 0.3 * s);
    for yy in (fy.ceil() as i32)..((fy + side) as i32) {
        for xx in (fx.ceil() as i32)..((fx + side) as i32) {
            cv.blend(xx, yy, BLACK, 0.55 * alpha);
        }
    }
    rect(cv, fx, fy);
}

/// Dessine une fenêtre. `text_of` donne le texte d'une ligne (déjà traduit le cas échéant) ;
/// les lignes que `skip` écarte (en attente de traduction) ne prennent pas de place ; celles que
/// `copy_of` désigne (messages de joueurs) finissent par l'icône de copie.
#[allow(clippy::too_many_arguments)]
pub fn render(
    book: &mut FontBook,
    v: &View,
    text_of: &dyn Fn(u16) -> Option<LineText>,
    skip: &dyn Fn(u16) -> bool,
    copy_of: &dyn Fn(u16) -> bool,
    copied: Option<u16>,
    metrics: Option<&FontMetrics>,
    now: Instant,
    opts: RenderOpts,
) -> (Canvas, Vec<Hit>) {
    let mut hits = Vec::new();
    let (w, h) = (v.width.max(1) as usize, v.height.max(1) as usize);
    let mut cv = Canvas::new(w, h);
    let px = v.font_px.max(4.0);
    let base = book.base(&v.font_path);
    let (ascent, natural) = book.line_metrics(base, px);
    let line_h = if v.line_h > 0.5 { v.line_h } else { natural };
    // Ligne de base : l'ascendante ramenée à la hauteur de ligne de WoW.
    let asc = ascent * line_h / natural.max(1.0);
    let pitch = line_h + v.spacing;
    let calage = v.flags & FLAG_CALAGE != 0;
    let (sx, sy, sa) = v.shadow;
    let outline = match v.font_flags & 3 {
        1 => 1,
        2 => 2,
        _ => 0,
    };

    let mut layout = Layout::new(book, base, px, line_h, metrics);
    if v.ui_k > 0.0 {
        layout.k = v.ui_k;
    }
    let mut bottom = h as f32;
    // Lignes dessinées par WoW dans la fenêtre : chaque ligne garde exactement sa place (celle
    // que WoW lui donne), pour ne pas chevaucher les siennes.
    let mixed = v.lines.iter().any(|(id, _)| *id == 0);
    let slot_of = |k: usize| -> usize { if mixed { v.rows.get(k).copied().unwrap_or(0) as usize } else { 0 } };
    let height_of = |k: f32| k * line_h + (k - 1.0) * v.spacing;
    // Lignes coupées à la largeur, police à sa taille (jamais réduite : 0.13.3, tailles égales).
    type Laid = Option<(Vec<Vec<(f32, Item)>>, Rgb)>;
    let laid: Vec<Laid> = v
        .lines
        .iter()
        .map(|(id, _)| {
            if *id == 0 || skip(*id) {
                return None;
            }
            let lt = text_of(*id).unwrap_or(LineText { color: [140, 140, 140], text: "…".into() });
            let mut items = parse_markup(&lt.text, lt.color);
            if copy_of(*id) {
                items.push(Item::Char(' ', lt.color));
                items.push(Item::Copy);
            }
            let advs = advances(book, base, px, &items, metrics, &layout);
            Some((wrap(&items, &advs, w as f32, layout.indent), lt.color))
        })
        .collect();
    // Fenêtre mêlée de lignes de WoW (0.13.3) : les lignes de WoW gardent exactement leur place ;
    // entre deux d'entre elles, les lignes des joueurs se partagent la place que WoW leur donne,
    // chacune sur autant de lignes qu'il lui faut, les plus récentes en bas. Ce qui ne tient pas :
    // les plus anciennes du groupe ne sont pas dessinées (la plus récente est coupée au besoin).
    // `plan[i]` : lignes de texte à dessiner (None : pas dessinée) ; `extra[i]` : lignes vides
    // laissées au-dessus pour que la ligne de WoW suivante reste à sa place.
    let n = v.lines.len();
    let mut plan: Vec<Option<usize>> = vec![None; n];
    let mut extra = vec![0usize; n];
    let anchor = |i: usize| v.lines[i].0 == 0 || (skip(v.lines[i].0) && slot_of(i) > 0);
    if mixed {
        let mut i = 0;
        while i < n {
            if anchor(i) {
                i += 1;
                continue;
            }
            let mut j = i;
            let mut room = 0usize;
            while j < n && !anchor(j) {
                if laid[j].is_some() {
                    room += slot_of(j).max(1);
                }
                j += 1;
            }
            let mut used = 0usize;
            let mut last = None;
            for (k, l) in laid.iter().enumerate().take(j).skip(i) {
                let Some((rows, _)) = l else { continue };
                let need = rows.len().max(1);
                if used + need <= room {
                    plan[k] = Some(need);
                } else if used == 0 {
                    plan[k] = Some(room);
                } else {
                    break;
                }
                used += plan[k].unwrap();
                last = Some(k);
            }
            if let Some(k) = last {
                extra[k] = room.saturating_sub(used);
            }
            i = j;
        }
    }
    // Place de chaque ligne dessinée : (indice, lignes de texte, haut, opacité).
    type Placed = (usize, Vec<Vec<(f32, Item)>>, f32, f32);
    let mut placed: Vec<Placed> = Vec::new();
    for (li, (_, age0)) in v.lines.iter().enumerate() {
        if bottom <= 0.0 {
            break;
        }
        let slot = slot_of(li);
        if anchor(li) {
            // Place laissée à WoW, ou réservée à une ligne qui attend sa traduction.
            bottom -= height_of(slot.max(1) as f32) + v.spacing;
            continue;
        }
        let Some((mut rows, color)) = laid[li].clone() else { continue };
        if mixed {
            match plan[li] {
                Some(k) if k < rows.len() => {
                    rows.truncate(k);
                    // Coupée : points de suspension au bout.
                    if let Some(row) = rows.last_mut() {
                        if let Some((x, _)) = row.pop() {
                            row.push((x, Item::Char('…', color)));
                        }
                    }
                }
                Some(_) => {}
                None => continue,
            }
        }
        let alpha = v.alpha(*age0, now);
        let k = rows.len().max(1) as f32;
        let top = bottom - height_of(k);
        placed.push((li, rows, top, alpha));
        bottom = top - v.spacing - extra[li] as f32 * pitch;
    }
    // Dispositif télévision (0.14) : le texte du jeu reste dessous. Une plaque sombre, sur toute
    // la largeur de la fenêtre, couvre les lignes des joueurs à la place que WoW leur donne et
    // celles redessinées ; elle s'efface avec elles.
    if let (Some((pc, po)), false) = (opts.plate, calage) {
        let mut band = vec![0f32; h];
        let pad = v.spacing / 2.0 + 1.0;
        let mut fill = |top: f32, bot: f32, a: f32| {
            let y0 = (top - pad).floor().max(0.0) as usize;
            let y1 = ((bot + pad).ceil().max(0.0) as usize).min(h);
            for b in band.iter_mut().take(y1).skip(y0) {
                *b = b.max(a);
            }
        };
        let mut ob = h as f32;
        for (li, (id, age0)) in v.lines.iter().enumerate() {
            if ob <= 0.0 {
                break;
            }
            let k = match v.rows.get(li).copied().unwrap_or(0) {
                0 => laid[li].as_ref().map_or(1, |(r, _)| r.len().max(1)),
                r => r as usize,
            };
            let top = ob - height_of(k as f32);
            if *id != 0 {
                fill(top, ob, po * v.alpha(*age0, now));
            }
            ob = top - v.spacing;
        }
        for (_, rows, top, alpha) in &placed {
            fill(*top, *top + height_of(rows.len().max(1) as f32), po * alpha);
        }
        for (y, a) in band.iter().enumerate() {
            if *a > 0.0 {
                for x in 0..w {
                    cv.blend(x as i32, y as i32, pc, *a);
                }
            }
        }
    }
    for (li, rows, top, alpha) in placed {
        let id = &v.lines[li].0;
        let scale = 1.0f32;
        let lpx = px * scale;
        if alpha > 0.0 {
            for (j, row) in rows.iter().enumerate() {
                let baseline = (top + j as f32 * pitch + asc + opts.dy).round() as i32;
                if (baseline as f32) < -line_h || baseline as f32 > h as f32 + line_h {
                    continue;
                }
                for (x, it) in row {
                    let x = x + opts.dx;
                    match it {
                        Item::Char(c, col) => {
                            let Some(f) = book.pick(base, *c) else { continue };
                            let (m, bmp) = book.glyph(f, *c, lpx).clone();
                            let gx = (x + m.xmin as f32).round() as i32;
                            let gy = baseline - (m.ymin + m.height as i32);
                            let pass = |cv: &mut Canvas, ox: i32, oy: i32, color: Rgb, a: f32| {
                                for by in 0..m.height {
                                    for bx in 0..m.width {
                                        let cov = bmp[by * m.width + bx];
                                        if cov > 0 {
                                            cv.blend(
                                                gx + bx as i32 + ox,
                                                gy + by as i32 + oy,
                                                color,
                                                a * cov as f32 / 255.0,
                                            );
                                        }
                                    }
                                }
                            };
                            if calage {
                                pass(&mut cv, 0, 0, MAGENTA, 0.8 * alpha);
                                continue;
                            }
                            if outline > 0 {
                                for (ox, oy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                                    pass(&mut cv, ox * outline, oy * outline, BLACK, alpha);
                                }
                            }
                            if sa > 0 && (sx != 0.0 || sy != 0.0) {
                                pass(&mut cv, sx.round() as i32, -sy.round() as i32, BLACK, alpha * sa as f32 / 255.0);
                            }
                            pass(&mut cv, 0, 0, *col, alpha);
                        }
                        Item::Icon(r) => {
                            // Centrée sur la ligne, décalée comme dans WoW (dy vers le haut).
                            let (iw, ih) = crate::icons::size_px(r, lpx, layout.k * scale);
                            let (iw, ih) = (iw.round().max(1.0) as usize, ih.round().max(1.0) as usize);
                            let center = baseline as f32 - asc + line_h * 0.5 - r.dy * layout.k * scale;
                            let x0 = (x + r.dx * layout.k * scale).round() as i32;
                            let y0 = (center - ih as f32 * 0.5).round() as i32;
                            let img = crate::icons::image_or_shape(r.id, iw, ih);
                            for yy in 0..ih {
                                for xx in 0..iw {
                                    let p = img.px[yy * iw + xx];
                                    if p[3] > 0 {
                                        let c = if calage { MAGENTA } else { [p[0], p[1], p[2]] };
                                        cv.blend(x0 + xx as i32, y0 + yy as i32, c, alpha * p[3] as f32 / 255.0);
                                    }
                                }
                            }
                        }
                        Item::Copy => {
                            let s = copy_size(lpx);
                            let x0 = x.round() as i32 + 1;
                            let y0 = baseline - (lpx * 0.78).round() as i32;
                            draw_copy(&mut cv, x0, y0, s, copied == Some(*id), calage, alpha);
                            if alpha > 0.05 {
                                hits.push(Hit { line: *id, x: x0 - 3, y: y0 - 3, w: s + 6, h: s + 6 });
                            }
                        }
                        Item::Break => {}
                    }
                }
            }
        }
    }
    (cv, hits)
}

/// Rectangles posés sur le chat (pixels de l'écran depuis le bas à gauche) : rien n'est dessiné
/// dessous, et leurs icônes de copie ne sont plus cliquables.
fn mask(cv: &mut Canvas, hits: &mut Vec<Hit>, v: &View, occ: &[(i32, i32, i32, i32)]) {
    let h = cv.h as i32;
    for &(l, b, w, rh) in occ {
        let x0 = (l - v.left).clamp(0, cv.w as i32);
        let x1 = (l + w - v.left).clamp(0, cv.w as i32);
        let y0 = (v.bottom + h - (b + rh)).clamp(0, h);
        let y1 = (v.bottom + h - b).clamp(0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                cv.px[y as usize * cv.w + x as usize] = 0;
            }
        }
        hits.retain(|t| t.x + t.w <= x0 || t.x >= x1 || t.y + t.h <= y0 || t.y >= y1);
    }
}

// --- État du miroir ----------------------------------------------------------------

#[cfg_attr(not(windows), allow(dead_code))]
pub struct Update {
    pub frame: u8,
    /// Coin bas-gauche en pixels physiques (origine en bas à gauche de l'écran).
    pub left: i32,
    pub bottom: i32,
    /// `None` : cacher la fenêtre.
    pub canvas: Option<Canvas>,
    /// Icônes de copie cliquables (pixels de l'image).
    pub hits: Vec<Hit>,
}

struct FrameState {
    view: View,
    dirty: bool,
    alpha_sig: Vec<u8>,
    shown: bool,
    hits: Vec<Hit>,
}

/// Message de joueur relayé par l'addon : ses lignes portent l'icône de copie.
#[derive(Clone, Debug)]
struct Chat {
    body: String,
    sender: String,
    at: Instant,
}

/// Traduction d'un message reçu : affichée à la place du texte d'origine, dans les lignes de
/// son expéditeur.
#[derive(Clone, Debug)]
struct Translated {
    body: String,
    sender: String,
    shown: String,
    at: Instant,
}

#[derive(Clone, Debug)]
struct Held {
    body: String,
    sender: String,
    until: Instant,
}

/// Une ligne neuve attend ce temps-là, au plus, de savoir si elle est à traduire (la langue est
/// connue dès que le message relayé arrive, en général en moins de 0,2 s).
pub const GRACE: Duration = Duration::from_millis(350);
/// Une ligne à traduire attend sa traduction ce temps-là au plus ; ensuite elle s'affiche telle
/// quelle, et la traduction la remplace à son arrivée (0.12.4).
pub const HOLD_MAX: Duration = Duration::from_secs(4);

/// Le texte d'une ligne cite-t-il ce message de cet expéditeur ? (Les lignes de l'addon qui
/// citent le message, « Envoyé sur… », ne sont pas concernées.)
fn line_has(text: &str, body: &str, sender: &str) -> bool {
    if !text.contains(body) {
        return false;
    }
    let short = sender.split(['-', ' ']).next().unwrap_or(sender);
    short.is_empty() || text.contains(short)
}

pub struct Mirror {
    pub book: FontBook,
    lines: HashMap<u16, LineText>,
    /// Mesures du jeu par (police, taille en dixièmes de pixel).
    metrics: HashMap<(String, u16), FontMetrics>,
    frames: HashMap<u8, FrameState>,
    /// Messages relayés récemment (S2) : leur texte est repéré dans les lignes (démo).
    recent: VecDeque<(String, Instant)>,
    /// Traductions des messages reçus.
    translations: VecDeque<Translated>,
    /// Arrivée de chaque ligne (une ligne neuve attend de savoir si elle est à traduire).
    line_first: HashMap<u16, Instant>,
    /// Messages en cours de traduction : leurs lignes attendent.
    held: Vec<Held>,
    /// Messages qu'il n'y a pas à traduire (ou plus) : leurs lignes s'affichent tout de suite.
    settled: VecDeque<(String, Instant)>,
    /// Messages affichés en version d'origine faute de traduction à temps : une traduction qui
    /// arrive après ne change plus la ligne.
    /// Messages de joueurs relayés (30 min) : icône de copie au bout de leurs lignes.
    chats: VecDeque<Chat>,
    /// Bulles d'aide, menus et fenêtres du jeu posés sur le chat.
    occluders: Vec<(i32, i32, i32, i32)>,
    /// Autre application passée devant WoW (gauche, bas, largeur, hauteur depuis le bas à gauche).
    window_occ: Option<(i32, i32, i32, i32)>,
    /// Ligne copiée à l'instant : coche verte à la place de l'icône.
    copied: Option<(u16, Instant)>,
    pub demo: bool,
    pub opts: RenderOpts,
    pub views_seen: usize,
    pub lines_seen: usize,
}

impl Default for Mirror {
    fn default() -> Self {
        Self::new()
    }
}

impl Mirror {
    pub fn new() -> Mirror {
        Mirror {
            book: FontBook::new(),
            lines: HashMap::new(),
            metrics: HashMap::new(),
            frames: HashMap::new(),
            recent: VecDeque::new(),
            translations: VecDeque::new(),
            line_first: HashMap::new(),
            held: Vec::new(),
            settled: VecDeque::new(),
            chats: VecDeque::new(),
            occluders: Vec::new(),
            window_occ: None,
            copied: None,
            demo: false,
            opts: RenderOpts::default(),
            views_seen: 0,
            lines_seen: 0,
        }
    }

    /// Renvoie la description si elle change la géométrie ou la police (pour l'afficher).
    pub fn on_view(&mut self, v: View) -> Option<View> {
        self.views_seen += 1;
        let frame = v.frame;
        let report = match self.frames.get(&frame) {
            None => v.flags & FLAG_VISIBLE != 0,
            Some(old) => {
                let o = &old.view;
                (o.left, o.bottom, o.width, o.height, o.flags & !FLAG_FADE, o.font_path.as_str(), (o.font_px * 10.0) as i32)
                    != (v.left, v.bottom, v.width, v.height, v.flags & !FLAG_FADE, v.font_path.as_str(), (v.font_px * 10.0) as i32)
            }
        };
        let st = FrameState {
            view: v.clone(),
            dirty: true,
            alpha_sig: Vec::new(),
            shown: self.frames.get(&frame).is_some_and(|f| f.shown),
            hits: self.frames.get(&frame).map(|f| f.hits.clone()).unwrap_or_default(),
        };
        self.frames.insert(frame, st);
        report.then_some(v)
    }

    /// Mesures de police reçues ; renvoie un résumé à afficher.
    pub fn on_measures(&mut self, p: &[u8]) -> Result<String, String> {
        let (frame, size10, m) = parse_measures(p)?;
        let path = self
            .frames
            .get(&frame)
            .map(|f| f.view.font_path.clone())
            .ok_or_else(|| format!("mesures pour ChatFrame{frame}, encore inconnue"))?;
        let key = (font_key(&path), size10);
        let fm = self.metrics.entry(key.clone()).or_default();
        let summary = match m {
            Measures::Adv(v) => {
                let n = v.len();
                fm.adv.extend(v);
                format!("{n} caractères (total {})", fm.adv.len())
            }
            Measures::Kern(v) => {
                let s: Vec<String> = v.iter().take(8).map(|(a, b, k)| format!("{a}{b} {k:+.2}")).collect();
                let more = if v.len() > 8 { format!(" (+{})", v.len() - 8) } else { String::new() };
                fm.kern.extend(v.iter().map(|(a, b, k)| ((*a, *b), *k)));
                format!("crénage : {}{more}", s.join(", "))
            }
            Measures::Indent(x) => {
                fm.indent = Some(x);
                format!("retrait des lignes coupées : {x:.1} px")
            }
        };
        for f in self.frames.values_mut() {
            if (font_key(&f.view.font_path), (f.view.font_px * 10.0).round() as u16) == key {
                f.dirty = true;
            }
        }
        Ok(format!("{path} {:.1} px : {summary}", size10 as f32 / 10.0))
    }

    /// Règles de mise en page en vigueur pour une fenêtre (pour les afficher).
    pub fn layout_summary(&mut self, frame: u8) -> Option<String> {
        let v = self.frames.get(&frame)?.view.clone();
        let m = self.metrics.get(&(font_key(&v.font_path), (v.font_px * 10.0).round() as u16)).cloned();
        let base = self.book.base(&v.font_path);
        let l = Layout::new(&mut self.book, base, v.font_px, v.line_h, m.as_ref());
        Some(format!(
            "avances du jeu {} caractères, rapport police/jeu {:.3}, crénage {} paires, retrait {:.1} px{}",
            m.as_ref().map_or(0, |m| m.adv.len()),
            l.ratio,
            l.kern_pairs,
            l.indent,
            if m.as_ref().is_some_and(|m| m.indent.is_some()) { " (mesuré)" } else { " (estimé)" }
        ))
    }

    /// Texte reçu d'une ligne (pour les tests).
    #[cfg(test)]
    pub fn raw_line(&self, id: u16) -> Option<String> {
        self.lines.get(&id).map(|l| l.text.clone())
    }

    /// Texte d'une ligne ; vrai s'il est nouveau ou a changé (l'addon le renvoie toutes les 60 s).
    pub fn on_line(&mut self, id: u16, body: &str) -> bool {
        self.lines_seen += 1;
        self.line_first.entry(id).or_insert_with(Instant::now);
        let lt = parse_line_body(body);
        if self.lines.get(&id) == Some(&lt) {
            return false;
        }
        self.lines.insert(id, lt);
        // Les fenêtres qui citent cette ligne sont à redessiner.
        for f in self.frames.values_mut() {
            if f.view.lines.iter().any(|(l, _)| *l == id) {
                f.dirty = true;
            }
        }
        true
    }

    /// Tout est à redessiner, et à remontrer (fenêtres cachées entre-temps).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn redraw_all(&mut self) {
        for f in self.frames.values_mut() {
            f.dirty = true;
            f.shown = false;
        }
    }

    pub fn note_chat(&mut self, body: &str, sender: &str, now: Instant) {
        if body.chars().count() >= 2 {
            self.recent.push_front((body.to_string(), now));
            self.recent.truncate(50);
        }
        if !body.trim().is_empty() {
            self.chats.push_front(Chat { body: body.to_string(), sender: sender.to_string(), at: now });
            self.chats.truncate(400);
        }
        for f in self.frames.values_mut() {
            f.dirty = true;
        }
    }

    /// Rectangles posés sur le chat (trame de l'addon) ; vrai s'ils ont changé.
    pub fn set_occluders(&mut self, p: &[u8]) -> bool {
        let n = p.first().copied().unwrap_or(0) as usize;
        let u = |i: usize| p.get(i..i + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as i32);
        let rects: Vec<(i32, i32, i32, i32)> = (0..n)
            .filter(|j| p.len() > 8 * (j + 1))
            .map(|j| {
                let o = 1 + 8 * j;
                (u(o), u(o + 2), u(o + 4), u(o + 6))
            })
            .collect();
        if rects == self.occluders {
            return false;
        }
        self.occluders = rects;
        self.dirty_all();
        true
    }

    /// Fenêtre d'une autre application devant WoW (rien n'est dessiné dessous) ; `None` : WoW
    /// actif. Vrai si ça change quelque chose.
    pub fn set_window_occluder(&mut self, r: Option<(i32, i32, i32, i32)>) -> bool {
        if r == self.window_occ {
            return false;
        }
        self.window_occ = r;
        self.dirty_all();
        true
    }

    /// Message relayé dont cette ligne est l'affichage.
    fn chat_of(&self, lt: &LineText) -> Option<&Chat> {
        self.chats.iter().find(|c| line_has(&lt.text, &c.body, &c.sender))
    }

    /// Texte copié pour une ligne : le message tel qu'il s'affiche (traduit le cas échéant), sans
    /// l'heure, le canal, le nom ni l'acronyme de langue.
    pub fn copy_text(&self, id: u16) -> Option<String> {
        let lt = self.lines.get(&id)?;
        let c = self.chat_of(lt)?;
        match self.translations.iter().find(|t| line_has(&lt.text, &t.body, &t.sender)) {
            Some(t) => Some(plain(strip_tag(&t.shown))),
            None => Some(plain(&c.body)),
        }
    }

    /// Clic dans la fenêtre des icônes de `frame` (pixels de l'image) : ligne et texte à copier.
    pub fn hit(&self, frame: u8, x: i32, y: i32) -> Option<(u16, String)> {
        let f = self.frames.get(&frame)?;
        let h = f.hits.iter().find(|h| h.contains(x, y))?;
        Some((h.line, self.copy_text(h.line)?))
    }

    /// Ligne copiée : coche verte pendant 1,5 s.
    pub fn mark_copied(&mut self, line: u16, now: Instant) {
        self.copied = Some((line, now));
        self.dirty_all();
    }

    /// Le message `body` de `sender` part à la traduction : ses lignes attendent (au plus
    /// `HOLD_MAX`). Faux si une de ses lignes est déjà affichée : elle n'attend pas, la traduction
    /// la remplacera à son arrivée (0.12.4 : tout message étranger est traduit).
    pub fn hold(&mut self, body: &str, sender: &str, now: Instant) -> bool {
        if body.trim().is_empty() {
            return false;
        }
        let shown = self.lines.iter().any(|(id, lt)| {
            line_has(&lt.text, body, sender) && self.line_first.get(id).is_some_and(|t| now.saturating_duration_since(*t) >= GRACE)
        });
        if shown {
            return false;
        }
        self.held.push(Held { body: body.to_string(), sender: sender.to_string(), until: now + HOLD_MAX });
        self.dirty_all();
        true
    }

    /// Rien à traduire (même langue, jargon, échec) : les lignes s'affichent.
    pub fn settle(&mut self, body: &str, now: Instant) {
        self.held.retain(|h| h.body != body);
        self.settled.push_front((body.to_string(), now));
        self.settled.truncate(200);
        self.dirty_all();
    }

    fn dirty_all(&mut self) {
        for f in self.frames.values_mut() {
            f.dirty = true;
        }
    }

    /// Le message `body` de `sender` s'affiche désormais `shown` (« [DE] traduction ») dans ses
    /// lignes ; le début de la ligne (heure, canal, nom) ne change pas. Ligne déjà affichée telle
    /// quelle (traduction arrivée après `HOLD_MAX`) : elle est remplacée (0.12.4).
    pub fn set_translation(&mut self, body: &str, sender: &str, shown: &str, now: Instant) -> bool {
        if body.trim().is_empty() {
            return false;
        }
        self.held.retain(|h| h.body != body);
        self.translations.retain(|t| t.body != body);
        self.translations.push_front(Translated {
            body: body.to_string(),
            sender: sender.to_string(),
            shown: shown.to_string(),
            at: now,
        });
        self.translations.truncate(200);
        self.dirty_all();
        true
    }

    /// Une ligne attend-elle encore (traduction en cours, ou langue pas encore connue) ?
    fn waiting(&self, id: u16, now: Instant) -> bool {
        let Some(lt) = self.lines.get(&id) else { return false };
        if self.translations.iter().any(|t| line_has(&lt.text, &t.body, &t.sender)) {
            return false;
        }
        if self.held.iter().any(|h| now < h.until && line_has(&lt.text, &h.body, &h.sender)) {
            return true;
        }
        if self.settled.iter().any(|(b, _)| lt.text.contains(b.as_str())) {
            return false;
        }
        // Ligne neuve : on attend de savoir (sauf en mode démonstration ou calage).
        !self.demo && self.line_first.get(&id).is_some_and(|t| now.saturating_duration_since(*t) < GRACE)
    }

    /// Lignes à ne pas dessiner encore : une ligne qui attend, et toutes les plus récentes
    /// (le chat garde son ordre, comme s'il arrivait un peu plus tard).
    fn hidden(&self, v: &View, now: Instant) -> Vec<u16> {
        if v.flags & FLAG_CALAGE != 0 {
            return Vec::new();
        }
        // Les lignes vont de la plus récente (en bas) à la plus ancienne.
        match v.lines.iter().rposition(|(id, _)| self.waiting(*id, now)) {
            Some(k) => v.lines[..=k].iter().map(|(id, _)| *id).collect(),
            None => Vec::new(),
        }
    }

    pub fn fonts_needed(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .frames
            .values()
            .filter(|f| f.view.drawn() && !self.book.knows(&f.view.font_path))
            .map(|f| f.view.font_path.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Texte affiché d'une ligne (pour les tests).
    #[cfg(test)]
    pub fn shown(&self, id: u16) -> Option<String> {
        self.text_for(id).map(|t| t.text)
    }

    fn text_for(&self, id: u16) -> Option<LineText> {
        let mut lt = self.lines.get(&id)?.clone();
        if let Some(t) = self.translations.iter().find(|t| line_has(&lt.text, &t.body, &t.sender)) {
            lt.text = lt.text.replacen(t.body.as_str(), &t.shown, 1);
            return Some(lt);
        }
        if self.demo {
            // Démonstration : le message du joueur reçoit l'acronyme de sa langue d'origine
            // (« ?? » en attendant la détection, S4) ; le reste de la ligne est inchangé.
            if let Some((body, _)) = self.recent.iter().find(|(b, _)| lt.text.contains(b.as_str())) {
                lt.text = lt.text.replacen(body.as_str(), &format!("[??] {body}"), 1);
            }
        }
        Some(lt)
    }

    /// Fenêtres à redessiner ou à cacher maintenant.
    pub fn tick(&mut self, now: Instant) -> Vec<Update> {
        // Image d'icône arrivée des archives du jeu : elle remplace sa forme provisoire.
        if crate::icons::take_changed() {
            self.dirty_all();
        }
        self.recent
            .retain(|(_, t)| now.saturating_duration_since(*t) < Duration::from_secs(120));
        // Une ligne de chat reste lisible en remontant l'historique : on garde 30 min.
        self.translations.retain(|t| now.saturating_duration_since(t.at) < Duration::from_secs(1800));
        // Traduction pas prête à temps : la ligne s'affiche telle quelle, en attendant qu'elle arrive.
        let expired: Vec<String> = self.held.iter().filter(|h| now >= h.until).map(|h| h.body.clone()).collect();
        for b in expired {
            self.held.retain(|h| h.body != b);
            self.settled.push_front((b, now));
        }
        self.settled.retain(|(_, t)| now.saturating_duration_since(*t) < Duration::from_secs(1800));
        self.chats.retain(|c| now.saturating_duration_since(c.at) < Duration::from_secs(1800));
        if self.copied.is_some_and(|(_, t)| now.saturating_duration_since(t) >= Duration::from_millis(1500)) {
            self.copied = None;
            self.dirty_all();
        }
        let known: Vec<u16> = self.lines.keys().copied().collect();
        self.line_first.retain(|id, _| known.contains(id));
        let mut ups = Vec::new();
        let frames: Vec<u8> = self.frames.keys().copied().collect();
        for fid in frames {
            let (drawn, sig, hidden) = {
                let f = &self.frames[&fid];
                let mut sig: Vec<u8> = f
                    .view
                    .lines
                    .iter()
                    .map(|(_, a)| (f.view.alpha(*a, now) * 32.0).round() as u8)
                    .collect();
                let hidden = self.hidden(&f.view, now);
                // Le nombre de lignes en attente fait partie de l'état dessiné.
                sig.push(hidden.len().min(255) as u8);
                (f.view.drawn(), sig, hidden)
            };
            let f = &self.frames[&fid];
            if !drawn {
                if f.shown {
                    ups.push(Update {
                        frame: fid,
                        left: f.view.left,
                        bottom: f.view.bottom,
                        canvas: None,
                        hits: Vec::new(),
                    });
                    let f = self.frames.get_mut(&fid).unwrap();
                    f.shown = false;
                    f.hits.clear();
                }
                continue;
            }
            if !f.dirty && f.alpha_sig == sig {
                continue;
            }
            let view = f.view.clone();
            let (mut canvas, mut hits) = {
                let me = &*self;
                let lines: HashMap<u16, LineText> = view
                    .lines
                    .iter()
                    .filter_map(|(id, _)| me.text_for(*id).map(|t| (*id, t)))
                    .collect();
                let copyable: Vec<u16> = view
                    .lines
                    .iter()
                    .filter(|(id, _)| me.lines.get(id).is_some_and(|lt| me.chat_of(lt).is_some()))
                    .map(|(id, _)| *id)
                    .collect();
                let copied = self.copied.map(|(l, _)| l);
                let demo = self.demo;
                let mut opts = self.opts;
                if view.flags & FLAG_COVER != 0 {
                    opts.plate = Some((PLATE, PLATE_ALPHA));
                }
                let metrics = self
                    .metrics
                    .get(&(font_key(&view.font_path), (view.font_px * 10.0).round() as u16))
                    .cloned();
                render(
                    &mut self.book,
                    &view,
                    &|id| lines.get(&id).cloned(),
                    &|id| hidden.contains(&id),
                    &|id| !demo && copyable.contains(&id),
                    copied,
                    metrics.as_ref(),
                    now,
                    opts,
                )
            };
            let mut occ = self.occluders.clone();
            occ.extend(self.window_occ);
            mask(&mut canvas, &mut hits, &view, &occ);
            let f = self.frames.get_mut(&fid).unwrap();
            f.dirty = false;
            f.alpha_sig = sig;
            f.shown = true;
            f.hits = hits.clone();
            ups.push(Update {
                frame: fid,
                left: view.left,
                bottom: view.bottom,
                canvas: Some(canvas),
                hits,
            });
        }
        ups
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Description au format de Mirror.lua (pour les tests).
    pub fn view_payload(frame: u8, flags: u8, geo: [u16; 4], path: &str, lines: &[(u16, u16)]) -> Vec<u8> {
        let mut p = vec![frame, flags];
        for g in geo {
            p.extend_from_slice(&g.to_be_bytes());
        }
        for v in [140u16, 160, 0] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.extend_from_slice(&[10, (-10i8) as u8, 255, 0]);
        p.extend_from_slice(&1200u16.to_be_bytes());
        p.extend_from_slice(&30u16.to_be_bytes());
        p.push(path.len() as u8);
        p.extend_from_slice(path.as_bytes());
        p.push(lines.len() as u8);
        for (id, age) in lines {
            p.extend_from_slice(&id.to_be_bytes());
            p.extend_from_slice(&age.to_be_bytes());
        }
        p
    }

    #[test]
    fn view_roundtrip_and_fading() {
        let t0 = Instant::now();
        let p = view_payload(1, FLAG_VISIBLE | FLAG_FADE, [40, 160, 760, 330], "Fonts\\ARIALN.TTF", &[(7, 5), (6, 1195), (5, 1220)]);
        let v = parse_view(&p, t0).unwrap();
        assert_eq!((v.frame, v.left, v.bottom, v.width, v.height), (1, 40, 160, 760, 330));
        assert_eq!((v.font_px, v.line_h, v.spacing), (14.0, 16.0, 0.0));
        assert_eq!(v.shadow, (1.0, -1.0, 255));
        assert_eq!(v.font_path, "Fonts\\ARIALN.TTF");
        assert_eq!(v.lines.len(), 3);
        assert_eq!(v.alpha(0.5, t0), 1.0);
        assert!((v.alpha(121.5, t0) - 0.5).abs() < 1e-4);
        assert_eq!(v.alpha(123.0, t0), 0.0);
        assert!(v.drawn());
        // Fenêtre cachée : deux octets suffisent.
        let hidden = parse_view(&[3, 0], t0).unwrap();
        assert!(!hidden.drawn());
        assert!(parse_view(&p[..20], t0).is_err());
        let official = parse_view(&view_payload(1, FLAG_VISIBLE | FLAG_OFFICIAL, [0, 0, 10, 10], "", &[]), t0).unwrap();
        assert!(!official.drawn());
    }

    #[test]
    fn markup() {
        let w = [255, 255, 255];
        let s = |items: &[Item]| -> String {
            items
                .iter()
                .map(|i| match i {
                    Item::Char(c, _) => *c,
                    Item::Icon(_) | Item::Copy => '#',
                    Item::Break => '/',
                })
                .collect()
        };
        let link = "a |cffa335ee|Hitem:19019::::|h[Tonnerre]|h|r b";
        let items = parse_markup(link, w);
        assert_eq!(s(&items), "a [Tonnerre] b");
        assert_eq!(items[2], Item::Char('[', [0xa3, 0x35, 0xee]));
        assert_eq!(items[13], Item::Char('b', w));
        assert_eq!(s(&parse_markup("x |TInterface\\Icon:0|t y || z|nw", w)), "x # y | z/w");
        assert_eq!(s(&parse_markup("|Kq12|k dit", w)), "??? dit");
        assert_eq!(s(&parse_markup("3 |4objet:objets; |cnGREEN_FONT_COLOR:ok|r", w)), "3 objet ok");
        assert_eq!(parse_markup("|cnGREEN_FONT_COLOR:o|r", w)[0], Item::Char('o', [25, 255, 25]));
        // Retail : qualité des objets et couleurs de classe nommées (0.12.3).
        assert_eq!(parse_markup("|cnIQ4:[Fiole]|r", w)[1], Item::Char('F', [163, 53, 238]));
        assert_eq!(parse_markup("|cnCLASS_MAGE:M|r", w)[0], Item::Char('M', [63, 199, 235]));
        match parse_markup("a|A:Professions-ChatIcon-Quality-Tier3:17:15::1|ab", w)[1] {
            Item::Icon(r) => assert_eq!((r.h, r.w, r.dy), (17.0, 15.0, 1.0)),
            other => panic!("{other:?}"),
        }
        // Balise incomplète : affichée telle quelle, sans planter.
        assert_eq!(s(&parse_markup("|c12", w)), "|c12");
        assert_eq!(s(&parse_markup("fin |H", w)), "fin ");
    }

    #[test]
    fn wrapping_like_wow() {
        let w = [0, 0, 0];
        let items = parse_markup("aaa bb cccc dddddddddddd e", w);
        let ones = |items: &[Item]| -> Vec<f32> {
            items.iter().map(|it| if matches!(it, Item::Char(..)) { 1.0 } else { 0.0 }).collect()
        };
        let rows = wrap(&items, &ones(&items), 8.0, 0.0);
        let text: Vec<String> = rows
            .iter()
            .map(|r| r.iter().map(|(_, it)| if let Item::Char(c, _) = it { *c } else { '#' }).collect())
            .collect();
        assert_eq!(text, vec!["aaa bb", "cccc", "dddddddd", "dddd e"]);
        // Abscisses : début de ligne à 0, puis cumul des avances.
        assert_eq!(rows[0][4].0, 4.0);
        assert_eq!(rows[1][0].0, 0.0);
        let it2 = parse_markup("un|ndeux", w);
        let rows = wrap(&it2, &ones(&it2), 100.0, 0.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(wrap(&[], &[], 10.0, 0.0).len(), 1);
        // Retrait des lignes coupées : la 2e ligne commence à 2, et a donc moins de place.
        let rows = wrap(&items, &ones(&items), 8.0, 2.0);
        let text: Vec<String> = rows
            .iter()
            .map(|r| r.iter().map(|(_, it)| if let Item::Char(c, _) = it { *c } else { '#' }).collect())
            .collect();
        assert_eq!(text, vec!["aaa bb", "cccc", "dddddd", "dddddd", "e"]);
        assert_eq!(rows[1][0].0, 2.0);
    }

    fn test_font() -> Option<Vec<u8>> {
        [
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
    }

    #[test]
    fn render_places_lines_from_the_bottom() {
        let Some(font) = test_font() else { return };
        let t0 = Instant::now();
        let mut m = Mirror::new();
        m.book.set_primary("Fonts\\ARIALN.TTF", Some(font)).unwrap();
        m.on_line(1, "FFFFFF\u{1F}Bonjour à tous");
        m.on_line(2, "40FF40\u{1F}[Guilde] Hans : Guten Abend");
        let p = view_payload(1, FLAG_VISIBLE, [10, 20, 300, 80], "fonts/arialn.ttf", &[(2, 0), (1, 0)]);
        assert!(m.on_view(parse_view(&p, t0).unwrap()).is_some());
        // Lignes neuves : elles attendent un instant de savoir si elles sont à traduire.
        let t0 = Instant::now() + GRACE;
        let ups = m.tick(t0);
        assert_eq!(ups.len(), 1);
        let cv = ups[0].canvas.as_ref().unwrap();
        let rows_with_ink: Vec<usize> = (0..cv.h)
            .filter(|y| (0..cv.w).any(|x| cv.px[y * cv.w + x] >> 24 > 128))
            .collect();
        // Deux lignes de 16 px depuis le bas : encre entre 48 et 80, rien au-dessus.
        assert!(rows_with_ink.iter().all(|&y| y >= 44), "{rows_with_ink:?}");
        assert!(rows_with_ink.iter().any(|&y| y >= 64) && rows_with_ink.iter().any(|&y| y < 64));
        // Couleur verte présente (ligne du bas).
        assert!(cv.px.iter().any(|p| (p >> 8) & 0xFF > 200 && (p >> 16) & 0xFF < 100 && p >> 24 > 200));
        // Rien n'a changé : pas de nouveau rendu.
        assert!(m.tick(t0).is_empty());
        // Ligne mise à jour : nouveau rendu ; fenêtre cachée : ordre de cacher.
        m.on_line(1, "FFFFFF\u{1F}Salut");
        assert_eq!(m.tick(t0).len(), 1);
        m.on_view(parse_view(&[1, 0], t0).unwrap());
        let ups = m.tick(t0);
        assert!(ups.len() == 1 && ups[0].canvas.is_none());
    }

    #[test]
    fn lines_left_to_wow_keep_their_place() {
        let Some(font) = test_font() else { return };
        let t0 = Instant::now();
        let mut m = Mirror::new();
        m.book.set_primary("Fonts\\ARIALN.TTF", Some(font)).unwrap();
        // Ligne de joueur longue (deux lignes chez nous) dans une place d'une ligne chez WoW.
        m.on_line(1, "FFFFFF\u{1F}[Hans] : Guten Abend zusammen, wer kommt heute mit nach Schwarzfels und danach noch in die Tiefen?");
        // De bas en haut : butin (WoW, 1 ligne), Hans (1 ligne), système (WoW, 2 lignes).
        let p = view_payload(1, FLAG_VISIBLE | FLAG_ROWS, [10, 20, 300, 80], "Fonts\\ARIALN.TTF", &[(0, 4096), (1, 4096), (0, 8192)]);
        let v = parse_view(&p, t0).unwrap();
        assert_eq!(v.rows, vec![1, 1, 2]);
        assert_eq!(v.lines[1], (1, 0.0));
        m.on_view(v);
        let ups = m.tick(Instant::now() + GRACE);
        let cv = ups[0].canvas.as_ref().unwrap();
        let ink: Vec<usize> = (0..cv.h).filter(|y| (0..cv.w).any(|x| cv.px[y * cv.w + x] >> 24 > 128)).collect();
        // Rien sur la place des lignes de WoW (16 px en bas, 32 px au-dessus de Hans) ; police à
        // sa taille (0.13.3), Hans coupé à une ligne (un jambage peut toucher le pixel du bas).
        assert!(!ink.is_empty());
        assert!(ink.iter().all(|&y| (44..=64).contains(&y)), "{ink:?}");
    }

    #[test]
    fn tv_cover_plates_hide_the_game_text_under_players_lines() {
        let Some(font) = test_font() else { return };
        let t0 = Instant::now();
        let mut m = Mirror::new();
        m.book.set_primary("Fonts\\ARIALN.TTF", Some(font)).unwrap();
        m.on_line(1, "FFFFFF\u{1F}[Hans] : Guten Abend");
        // De bas en haut : Hans (1 ligne chez WoW), système (WoW, 2 lignes). Dispositif télévision :
        // le texte de Hans reste dans le jeu, une plaque le couvre ; la ligne de WoW reste à nu.
        let geo = [10, 20, 300, 80];
        let lines = [(1, 4096), (0, 8192)];
        let p = view_payload(1, FLAG_VISIBLE | FLAG_ROWS | FLAG_COVER, geo, "Fonts\\ARIALN.TTF", &lines);
        m.on_view(parse_view(&p, t0).unwrap());
        let ups = m.tick(Instant::now() + GRACE);
        let cv = ups[0].canvas.as_ref().unwrap();
        let a = |x: usize, y: usize| cv.px[y * cv.w + x] >> 24;
        // Bord droit, sans texte : plaque presque opaque sur la place de Hans (64..80), rien sur
        // celle de la ligne de WoW (32..64), ni au-dessus.
        assert!((235..=245).contains(&a(295, 72)), "{}", a(295, 72));
        assert!((235..=245).contains(&a(295, 65)), "{}", a(295, 65));
        assert_eq!(a(295, 50), 0);
        assert_eq!(a(295, 10), 0);
        // Sans le dispositif : pas de plaque.
        let p = view_payload(1, FLAG_VISIBLE | FLAG_ROWS, geo, "Fonts\\ARIALN.TTF", &lines);
        m.on_view(parse_view(&p, t0).unwrap());
        let ups = m.tick(Instant::now() + GRACE);
        let cv = ups[0].canvas.as_ref().unwrap();
        assert_eq!(cv.px[72 * cv.w + 295] >> 24, 0);
    }

    #[test]
    fn players_lines_share_their_room_at_full_size() {
        let Some(font) = test_font() else { return };
        let t0 = Instant::now();
        let mut m = Mirror::new();
        m.book.set_primary("Fonts\\ARIALN.TTF", Some(font)).unwrap();
        // Entre deux lignes de WoW : Hans (traduction longue, 2 lignes) et Olga (courte) ont chacun
        // une place d'une ligne chez WoW : 2 lignes en tout. Hans, le plus récent, les prend ;
        // Olga, plus ancienne, n'est pas dessinée ; la police garde sa taille.
        m.on_line(1, "FFFFFF\u{1F}[Hans] : Guten Abend zusammen, wer kommt heute mit nach Schwarzfels und danach noch in die Tiefen?");
        m.on_line(2, "FFFFFF\u{1F}[Olga] : Hallo");
        let p = view_payload(1, FLAG_VISIBLE | FLAG_ROWS, [10, 20, 300, 96], "Fonts\\ARIALN.TTF", &[(0, 4096), (1, 4096), (2, 4096), (0, 4096)]);
        m.on_view(parse_view(&p, t0).unwrap());
        let ups = m.tick(Instant::now() + GRACE);
        let cv = ups[0].canvas.as_ref().unwrap();
        let ink: Vec<usize> = (0..cv.h).filter(|y| (0..cv.w).any(|x| cv.px[y * cv.w + x] >> 24 > 128)).collect();
        // Lignes de WoW : 16 px en bas (80..96) et 16 px en haut (32..48) restent vides.
        assert!(ink.iter().all(|&y| (47..=80).contains(&y)), "{ink:?}");
        // Deux lignes de texte (Hans sur deux lignes) : de l'encre dans les deux bandes de 16 px.
        assert!(ink.iter().any(|&y| y < 62) && ink.iter().any(|&y| y > 66), "{ink:?}");
    }

    #[test]
    fn copy_icons_and_menus_over_the_chat() {
        let Some(font) = test_font() else { return };
        let t0 = Instant::now();
        let mut m = Mirror::new();
        m.book.set_primary("Fonts\\ARIALN.TTF", Some(font)).unwrap();
        m.on_line(1, "FFFFFF\u{1F}[Hans] : Guten Abend");
        m.on_line(2, "FFFF00\u{1F}Vous recevez l'objet : [Cache de trésors amani]");
        m.note_chat("Guten Abend", "Hans-Ysondre", t0);
        m.settle("Guten Abend", t0);
        let p = view_payload(1, FLAG_VISIBLE, [10, 20, 300, 80], "Fonts\\ARIALN.TTF", &[(2, 0), (1, 0)]);
        m.on_view(parse_view(&p, t0).unwrap());
        let now = Instant::now() + GRACE;
        let ups = m.tick(now);
        // Message de joueur : une icône au bout ; message du jeu : aucune.
        assert_eq!(ups[0].hits.len(), 1);
        let h = ups[0].hits[0];
        assert_eq!(h.line, 1);
        assert!(h.x > 60 && h.x < 200, "{h:?}");
        // Clic sur l'icône : le message, sans l'heure ni le nom.
        assert_eq!(m.hit(1, h.x + h.w / 2, h.y + h.h / 2), Some((1, "Guten Abend".to_string())));
        assert_eq!(m.hit(1, 0, 0), None);
        // Traduit : c'est la traduction qui est copiée, sans l'acronyme.
        m.set_translation("Guten Abend", "Hans-Ysondre", "[DE] Bonsoir", now);
        assert_eq!(m.copy_text(1).as_deref(), Some("Bonsoir"));
        // Menu ouvert sur le chat : rien n'est dessiné dessous, l'icône n'est plus cliquable.
        let mut occ = vec![1u8];
        for v in [10u16, 20, 300, 80] {
            occ.extend_from_slice(&v.to_be_bytes());
        }
        assert!(m.set_occluders(&occ));
        let ups = m.tick(now);
        assert!(ups[0].canvas.as_ref().unwrap().px.iter().all(|p| *p == 0));
        assert!(ups[0].hits.is_empty());
        assert!(!m.set_occluders(&occ), "même rectangle : rien à refaire");
        // Bulle d'aide sur le haut de la fenêtre seulement : le bas reste dessiné.
        let mut top = vec![1u8];
        for v in [10u16, 80, 300, 20] {
            top.extend_from_slice(&v.to_be_bytes());
        }
        m.set_occluders(&top);
        let cv = m.tick(now).remove(0).canvas.unwrap();
        assert!((0..20 * cv.w).all(|i| cv.px[i] == 0));
        assert!(cv.px[20 * cv.w..].iter().any(|p| *p != 0));
        // Autre application devant WoW, sur la moitié droite de la fenêtre : la gauche reste dessinée.
        m.set_occluders(&[0]);
        assert!(m.set_window_occluder(Some((160, 0, 1000, 1000))));
        let cv = m.tick(now).remove(0).canvas.unwrap();
        assert!((0..cv.h).all(|y| (150..cv.w).all(|x| cv.px[y * cv.w + x] == 0)));
        assert!(cv.px.iter().any(|p| *p != 0));
        assert!(m.set_window_occluder(None));
        // Menu fermé : l'icône revient.
        m.set_occluders(&occ);
        assert!(m.set_occluders(&[0]));
        assert_eq!(m.tick(now)[0].hits.len(), 1);
        // Clic : coche verte un instant, puis l'icône.
        m.mark_copied(1, now);
        assert_eq!(m.tick(now).len(), 1);
        assert!(m.tick(now).is_empty());
        assert_eq!(m.tick(now + Duration::from_millis(1600)).len(), 1);
    }

    #[test]
    fn measures_drive_the_layout() {
        let t0 = Instant::now();
        let mut m = Mirror::new();
        // Mesures reçues avant la description : refusées (police encore inconnue).
        let mut adv = vec![1u8, 0, 140, 0, 2];
        for (c, a) in [('é', 7.5f32), ('W', 12.0)] {
            let cp = c as u32;
            adv.extend_from_slice(&[(cp >> 16) as u8, (cp >> 8) as u8, cp as u8]);
            adv.extend_from_slice(&((a * 64.0) as u16).to_be_bytes());
        }
        assert!(m.on_measures(&adv).is_err());
        let p = view_payload(1, FLAG_VISIBLE, [0, 0, 100, 50], "Fonts\\ARIALN.TTF", &[]);
        m.on_view(parse_view(&p, t0).unwrap());
        assert!(m.on_measures(&adv).unwrap().contains("2 caractères"));
        let mut kern = vec![1u8, 0, 140, 1, 1, 0, 0, b'A', 0, 0, b'V'];
        kern.extend_from_slice(&(-64i16).to_be_bytes());
        assert!(m.on_measures(&kern).unwrap().contains("AV -1.00"));
        assert_eq!(m.metrics[&(font_key("fonts/arialn.ttf"), 140)].kern[&('A', 'V')], -1.0);
        assert!(m.on_measures(&[1, 0, 140, 2, 0x03, 0xC0]).unwrap().contains("15.0 px"));
        let fm = &m.metrics[&(font_key("fonts/arialn.ttf"), 140)];
        assert_eq!(fm.adv[&'é'], 7.5);
        assert_eq!(fm.indent, Some(15.0));
        assert!(m.on_measures(&[1, 0, 140, 9, 0]).is_err());
    }

    #[test]
    fn demo_marks_relayed_message() {
        let mut m = Mirror::new();
        m.demo = true;
        m.on_line(4, "FFFFFF\u{1F}[Hans] : Guten Abend");
        m.note_chat("Guten Abend", "Hans", Instant::now());
        assert_eq!(m.text_for(4).unwrap().text, "[Hans] : [??] Guten Abend");
    }

    #[test]
    fn translated_message_replaces_the_original_in_its_line() {
        let mut m = Mirror::new();
        m.on_line(5, "FFC0C0\u{1F}[2. Commerce] [Hans] : Suche Gruppe für die Todesminen");
        // Ligne de l'addon qui cite le même texte : pas touchée.
        m.on_line(6, "66CCFF\u{1F}[Chevaliers] Envoyé sur SAY : Suche Gruppe für die Todesminen");
        assert!(m.set_translation("Suche Gruppe für die Todesminen", "Hans-Ysondre", "[DE] Je cherche un groupe pour les Mortemines", Instant::now()));
        let t = m.text_for(5).unwrap();
        assert_eq!(t.text, "[2. Commerce] [Hans] : [DE] Je cherche un groupe pour les Mortemines");
        assert_eq!(t.color, [0xFF, 0xC0, 0xC0]);
        assert_eq!(m.text_for(6).unwrap().text, "[Chevaliers] Envoyé sur SAY : Suche Gruppe für die Todesminen");
    }

    #[test]
    fn a_line_waits_for_its_translation_and_keeps_the_chat_order() {
        let t0 = Instant::now();
        let mut m = Mirror::new();
        let geo = [32, 95, 430, 120];
        let view = |lines: &[(u16, u16)]| parse_view(&view_payload(1, FLAG_VISIBLE, geo, "Fonts\\ARIALN.TTF", lines), t0).unwrap();
        m.on_line(1, "FFFFFF\u{1F}[Pierre] : salut");
        m.settle("salut", t0);
        m.on_line(2, "FFFFFF\u{1F}[Hans] : Guten Abend zusammen");
        m.on_line(3, "FFFFFF\u{1F}[Paul] : ça va ?");
        m.settle("ça va ?", t0);
        // Les lignes vont de la plus récente à la plus ancienne : 3, 2, 1.
        let v = view(&[(3, 0), (2, 0), (1, 0)]);
        assert!(m.hold("Guten Abend zusammen", "Hans", t0));
        // La ligne de Hans attend, et celle de Paul, plus récente, aussi : l'ordre est gardé.
        assert_eq!(m.hidden(&v, t0), vec![3, 2]);
        assert!(m.set_translation("Guten Abend zusammen", "Hans", "[DE] Bonsoir à tous", t0));
        assert!(m.hidden(&v, t0).is_empty());
        assert_eq!(m.text_for(2).unwrap().text, "[Hans] : [DE] Bonsoir à tous");
        // Traduction lente : la ligne s'affiche telle quelle après 4 s, puis la traduction la
        // remplace à son arrivée (0.12.4).
        m.on_line(4, "FFFFFF\u{1F}[Olga] : Всем привет");
        assert!(m.hold("Всем привет", "Olga", t0));
        let v = view(&[(4, 0), (3, 0), (2, 0), (1, 0)]);
        assert_eq!(m.hidden(&v, t0), vec![4]);
        let later = t0 + HOLD_MAX + Duration::from_millis(10);
        m.tick(later);
        assert!(m.hidden(&v, later).is_empty());
        assert_eq!(m.text_for(4).unwrap().text, "[Olga] : Всем привет");
        assert!(m.set_translation("Всем привет", "Olga", "[RU] Salut à tous", later));
        assert_eq!(m.text_for(4).unwrap().text, "[Olga] : [RU] Salut à tous");
        // Ligne neuve dont la langue n'est pas encore connue : elle attend un instant au plus.
        m.on_line(5, "FFFFFF\u{1F}Le Commerce est fermé");
        let v = view(&[(5, 0), (4, 0)]);
        let now = Instant::now();
        assert_eq!(m.hidden(&v, now), vec![5]);
        assert!(m.hidden(&v, now + GRACE).is_empty());
        // Ligne déjà affichée : elle n'attend plus (sa traduction la remplacera).
        assert!(!m.hold("Le Commerce est fermé", "", now + GRACE));
        assert!(m.hidden(&v, now + GRACE).is_empty());
    }
}
