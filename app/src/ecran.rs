//! Écrans branchés (0.14) : nom (EDID), liaison (HDMI, DisplayPort...), HDR, taille physique.
//! Une télévision est reconnue avant la connexion du personnage ; l'addon applique alors le
//! dispositif télévision : le texte du chat du jeu n'est pas caché, le chat traduit le recouvre
//! (plaques sombres sous les lignes redessinées). Si rien ne peut s'afficher par-dessus le jeu sur
//! cet écran, le joueur garde le chat du jeu au lieu d'un chat vide.

/// Liaison vers l'écran (DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY).
pub fn link_label(tech: u32) -> &'static str {
    match tech {
        0 => "VGA",
        4 => "DVI",
        5 => "HDMI",
        6 => "LVDS",
        10 => "DisplayPort",
        11 => "DisplayPort interne",
        15 => "Miracast",
        0x8000_0000 => "écran intégré",
        _ => "autre liaison",
    }
}

pub const LINK_HDMI: u32 = 5;

/// HDR (« couleur avancée » de Windows).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Hdr {
    #[default]
    Unknown,
    No,
    Off,
    On,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screen {
    /// Périphérique de Windows, « \\.\DISPLAY2 ».
    pub gdi: String,
    /// Nom donné par l'écran (EDID), « LG TV SSCR2 ».
    pub name: String,
    pub link: u32,
    pub hdr: Hdr,
    /// Taille de l'image en centimètres (EDID), si l'écran la donne.
    pub size_cm: Option<(u32, u32)>,
    pub w: u32,
    pub h: u32,
    pub hz: u32,
    pub primary: bool,
}

/// Fabricants de télévisions dont l'EDID ne porte que le nom de marque.
const TV_BRANDS: &[&str] = &[
    "HISENSE", "TCL", "VIZIO", "PANASONIC", "SHARP", "TOSHIBA", "GRUNDIG", "METZ", "LOEWE", "THOMSON", "SKYWORTH", "FUNAI",
];
/// Marques qui font aussi des écrans de PC : télévision si le nom n'est que la marque, en HDMI.
const BOTH_BRANDS: &[&str] = &["SAMSUNG", "LG", "SONY", "PHILIPS", "XIAOMI", "MI"];

impl Screen {
    /// Diagonale en pouces, si l'écran donne sa taille.
    pub fn diagonal(&self) -> Option<f32> {
        let (w, h) = self.size_cm?;
        if w == 0 || h == 0 {
            return None;
        }
        Some(((w * w + h * h) as f32).sqrt() / 2.54)
    }

    /// Télévision : « TV » dans son nom, une marque de télévisions seule, ou une diagonale de
    /// 39 pouces et plus.
    pub fn is_tv(&self) -> bool {
        let up = self.name.to_uppercase();
        let words: Vec<&str> = up.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        if words.iter().any(|w| matches!(*w, "TV" | "FTV" | "HDTV" | "SMARTTV" | "LGTV" | "BRAVIA")) {
            return true;
        }
        if words.first().is_some_and(|w| TV_BRANDS.contains(w)) {
            return true;
        }
        if words.len() == 1 && BOTH_BRANDS.contains(&words[0]) && self.link == LINK_HDMI {
            return true;
        }
        self.diagonal().is_some_and(|d| d >= 39.0)
    }

    /// « \\.\DISPLAY2 « LG TV SSCR2 » : télévision, HDMI, HDR actif, 3840x2160 à 120 Hz, 65 pouces ».
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.is_tv() {
            parts.push("télévision".to_string());
        }
        parts.push(link_label(self.link).to_string());
        match self.hdr {
            Hdr::On => parts.push("HDR actif".into()),
            Hdr::Off => parts.push("HDR possible, coupé".into()),
            _ => {}
        }
        if self.w > 0 {
            parts.push(if self.hz > 0 { format!("{}x{} à {} Hz", self.w, self.h, self.hz) } else { format!("{}x{}", self.w, self.h) });
        }
        if let Some(d) = self.diagonal() {
            parts.push(format!("{:.0} pouces", d));
        }
        if self.primary {
            parts.push("principal".into());
        }
        let name = if self.name.is_empty() { "sans nom".to_string() } else { format!("« {} »", self.name) };
        format!("{} {name} : {}", self.gdi, parts.join(", "))
    }
}

/// Clé du registre où Windows garde l'EDID d'un écran, depuis son chemin de périphérique
/// (`\\?\DISPLAY#GSM5B09#5&1a2b3c&0&UID4352#{e6f07b5f-...}`).
pub fn edid_key(device_path: &str) -> Option<String> {
    let p = device_path.trim_start_matches("\\\\?\\");
    let mut it = p.split('#');
    let class = it.next()?;
    let model = it.next()?;
    let instance = it.next()?;
    if !class.eq_ignore_ascii_case("DISPLAY") || model.is_empty() || instance.is_empty() {
        return None;
    }
    Some(format!("SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{model}\\{instance}\\Device Parameters"))
}

/// Taille de l'image (cm) et nom de l'écran, lus dans son EDID.
pub type Edid = (Option<(u32, u32)>, Option<String>);

pub fn parse_edid(e: &[u8]) -> Option<Edid> {
    if e.len() < 128 || e[..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
        return None;
    }
    let size = (e[21] > 0 && e[22] > 0).then_some((e[21] as u32, e[22] as u32));
    let name = (0..4).map(|k| 54 + 18 * k).find_map(|o| {
        let d = &e[o..o + 18];
        (d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xFC).then(|| {
            let raw: Vec<u8> = d[5..].iter().copied().take_while(|c| *c != 0x0A && *c != 0).collect();
            String::from_utf8_lossy(&raw).trim().to_string()
        })
    });
    Some((size, name.filter(|n| !n.is_empty())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scr(name: &str, link: u32, size: Option<(u32, u32)>) -> Screen {
        Screen { gdi: "\\\\.\\DISPLAY2".into(), name: name.into(), link, size_cm: size, w: 3840, h: 2160, hz: 120, ..Default::default() }
    }

    #[test]
    fn televisions_are_recognised() {
        assert!(scr("LG TV SSCR2", LINK_HDMI, Some((160, 90))).is_tv());
        assert!(scr("SONY TV  *00", LINK_HDMI, None).is_tv());
        assert!(scr("Panasonic-TV", LINK_HDMI, None).is_tv());
        assert!(scr("PHILIPS FTV", LINK_HDMI, None).is_tv());
        assert!(scr("Hisense", LINK_HDMI, None).is_tv());
        assert!(scr("SAMSUNG", LINK_HDMI, None).is_tv());
        // Écrans de PC : nom de modèle, DisplayPort, petite diagonale.
        assert!(!scr("LG ULTRAGEAR", 10, Some((88, 37))).is_tv());
        assert!(!scr("SAMSUNG", 10, Some((60, 34))).is_tv());
        assert!(!scr("Odyssey G7", LINK_HDMI, Some((70, 39))).is_tv());
        // Taille seule : 55 pouces.
        assert!(scr("XYZ 55", LINK_HDMI, Some((121, 68))).is_tv());
        let d = scr("LG TV SSCR2", LINK_HDMI, Some((160, 90)));
        let text = Screen { hdr: Hdr::On, ..d }.describe();
        assert!(text.contains("« LG TV SSCR2 » : télévision, HDMI, HDR actif, 3840x2160 à 120 Hz, 72 pouces"), "{text}");
    }

    #[test]
    fn edid_name_size_and_registry_key() {
        assert_eq!(
            edid_key("\\\\?\\DISPLAY#GSM5B09#5&1a2b3c&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}").as_deref(),
            Some("SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\GSM5B09\\5&1a2b3c&0&UID4352\\Device Parameters")
        );
        assert_eq!(edid_key("\\\\?\\USB#xyz"), None);
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
        e[21] = 160;
        e[22] = 90;
        let d = 72;
        e[d + 3] = 0xFC;
        e[d + 5..d + 18].copy_from_slice(b"LG TV SSCR2\n ");
        assert_eq!(parse_edid(&e), Some((Some((160, 90)), Some("LG TV SSCR2".into()))));
        assert_eq!(parse_edid(&e[..100]), None);
    }
}
