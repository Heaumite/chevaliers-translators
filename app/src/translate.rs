//! Traduction simulée, en attendant le moteur choisi au test S4.
//! Elle marque le texte avec la langue cible pour que la boucle d'envoi soit visible en jeu.

/// Taille maximale d'un message de chat WoW, en octets.
pub const MAX_CHAT_BYTES: usize = 255;

pub fn translate(text: &str, dest: &str) -> String {
    truncate_utf8(&format!("[→{dest}] {text}"), MAX_CHAT_BYTES)
}

/// Coupe à `max` octets sans casser un caractère UTF-8.
pub fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Longueur d'un code de couleur en tête de `s` : « |cAARRGGBB » (10 caractères) ou, depuis les
/// clients récents, « |cnNOM: » (« |cnIQ4: » : qualité d'un objet ; 0.17.0).
pub fn color_code_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if !s.starts_with("|c") {
        return None;
    }
    if b.len() >= 10 && b[2..10].iter().all(|c| c.is_ascii_hexdigit()) {
        return Some(10);
    }
    if let Some(name) = s.strip_prefix("|cn") {
        let n = name.find(':')?;
        if n > 0 && n <= 45 && name.as_bytes()[..n].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            return Some(3 + n + 1);
        }
    }
    None
}

/// Liens d'objets, de sorts ou de quêtes d'un message (« |cffa335ee|Hitem:…|h[Nom]|h|r »,
/// « |cnIQ4:|Hitem:…|h[Nom]|h|r » ou sans couleur), dans l'ordre : positions de début et de fin.
pub fn link_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = s[from..].find("|H").map(|i| i + from) {
        // Fin des données du lien, puis fin du texte affiché.
        let Some(a) = s[i + 2..].find("|h").map(|a| a + i + 2) else { break };
        let Some(b) = s[a + 2..].find("|h").map(|b| b + a + 2) else { break };
        // Code de couleur collé devant le lien (ancien ou nouveau format) : il en fait partie
        // (0.17.0 : « |cnIQ3: » restait devant le repère collé, et le lien remis par l'addon
        // partait cassé : « Invalid escape code in chat message »).
        let mut start = i;
        let low = i.saturating_sub(48).max(from);
        if let Some(k) = s.as_bytes()[low..i].windows(2).rposition(|w| w == b"|c").map(|k| k + low) {
            if color_code_len(&s[k..]) == Some(i - k) {
                start = k;
            }
        }
        let mut end = b + 2;
        if s[end..].starts_with("|r") {
            end += 2;
        }
        out.push((start, end));
        from = end;
    }
    out
}

/// Codes d'affichage du jeu hors des liens (couleurs, « |r », icônes, « | » isolés) : retirés du
/// collage (0.17.0). Collés, ils deviendraient du texte brut, ou casseraient le lien que l'addon
/// remet à la place de son repère.
pub fn strip_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(r) = rest.strip_prefix('|') {
            let skip = if let Some(n) = color_code_len(rest) {
                n
            } else if r.starts_with('T') {
                rest.find("|t").map_or(1, |e| e + 2)
            } else if r.starts_with('A') {
                rest.find("|a").map_or(1, |e| e + 2)
            } else if r.starts_with('n') {
                out.push(' ');
                2
            } else if r.starts_with('r') || r.starts_with('|') {
                2
            } else {
                1
            };
            i += skip;
            continue;
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Texte à coller dans la boîte de saisie de WoW : un lien collé y deviendrait du texte brut (le
/// jeu protège les « | » collés), alors chaque lien devient « {n} », son rang dans le texte
/// d'origine ; l'addon remet les vrais liens juste avant l'envoi. Lien inconnu : son nom seul.
/// Autres codes du jeu : retirés (0.17.0).
pub fn paste_form(translated: &str, source: &str) -> String {
    let src: Vec<&str> = link_spans(source).into_iter().map(|(a, b)| &source[a..b]).collect();
    let mut out = String::with_capacity(translated.len());
    let mut last = 0;
    for (a, b) in link_spans(translated) {
        out.push_str(&strip_codes(&translated[last..a]));
        let link = &translated[a..b];
        match src.iter().position(|l| *l == link) {
            Some(k) => out.push_str(&format!("{{{}}}", k + 1)),
            None => out.push_str(&crate::mirror::plain(link)),
        }
        last = b;
    }
    out.push_str(&strip_codes(&translated[last..]));
    truncate_utf8(&out, MAX_CHAT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_become_markers_for_the_paste() {
        let thunder = "|cffa335ee|Hitem:19019::::::::60:::::|h[Thunderfury]|h|r";
        let rune = "|Hspell:123|h[Rune]|h";
        let src = format!("Je vends {thunder} et {rune}, mp");
        assert_eq!(link_spans(&src).len(), 2);
        // Ordre changé par la traduction : chaque lien garde son rang d'origine.
        let tr = format!("Verkaufe {rune} und {thunder}, flüstert");
        assert_eq!(paste_form(&tr, &src), "Verkaufe {2} und {1}, flüstert");
        // Lien absent du texte d'origine : son nom seul ; texte sans lien : inchangé.
        assert_eq!(paste_form(&format!("x {thunder}"), "x"), "x [Thunderfury]");
        assert_eq!(paste_form("Bonjour à tous", "Bonjour à tous"), "Bonjour à tous");
    }

    #[test]
    fn retail_quality_links_stay_whole_in_the_paste() {
        // 0.17.0 : lien du retail « |cnIQ3: » (rapport du 22/11/2026 : « ||Hitem:242756… »,
        // « Invalid escape code in chat message »).
        let fish = "|cnIQ3:|Hitem:242756::::::::90:70:::::::::|h[Roulés de poisson touchés par le Vide copieux]|h|r";
        let gem = "|cnIQ2:|Hitem:240888::::::::90:70:::::::::|h[Olivine rapide sans défaut |A:Professions-ChatIcon-Quality-12-Tier2:17:15::1|a]|h|r";
        assert_eq!(link_spans(fish), vec![(0, fish.len())]);
        assert_eq!(paste_form(fish, fish), "{1}");
        let src = format!("WTS {gem} and {fish}");
        assert_eq!(paste_form(&format!("Vends {fish} et {gem}"), &src), "Vends {2} et {1}");
        // Codes restés hors des liens : retirés, jamais collés.
        assert_eq!(paste_form("|cnIQ3:{1} |cff00ff00vert|r a|b", "x"), "{1} vert ab");
        assert_eq!(color_code_len("|cnIQ4:x"), Some(7));
        assert_eq!(color_code_len("|cffa335eex"), Some(10));
        assert_eq!(color_code_len("|cnIQ 4:"), None);
    }

    #[test]
    fn truncation_keeps_characters_whole() {
        let long = "é".repeat(200); // 400 octets
        let t = translate(&long, "DE");
        assert!(t.len() <= MAX_CHAT_BYTES);
        assert!(t.starts_with("[→DE] "));
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }
}
