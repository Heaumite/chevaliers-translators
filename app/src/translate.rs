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

/// Liens d'objets, de sorts ou de quêtes d'un message (« |cffa335ee|Hitem:…|h[Nom]|h|r » ou sans
/// couleur), dans l'ordre : positions de début et de fin.
pub fn link_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = s[from..].find("|H").map(|i| i + from) {
        // Fin des données du lien, puis fin du texte affiché.
        let Some(a) = s[i + 2..].find("|h").map(|a| a + i + 2) else { break };
        let Some(b) = s[a + 2..].find("|h").map(|b| b + a + 2) else { break };
        let mut start = i;
        if i >= 10 && s.is_char_boundary(i - 10) {
            let c = &s[i - 10..i];
            if c.starts_with("|c") && c[2..].chars().all(|ch| ch.is_ascii_hexdigit()) {
                start = i - 10;
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

/// Texte à coller dans la boîte de saisie de WoW : un lien collé y deviendrait du texte brut (le
/// jeu protège les « | » collés), alors chaque lien devient « {n} », son rang dans le texte
/// d'origine ; l'addon remet les vrais liens juste avant l'envoi. Lien inconnu : son nom seul.
pub fn paste_form(translated: &str, source: &str) -> String {
    let src: Vec<&str> = link_spans(source).into_iter().map(|(a, b)| &source[a..b]).collect();
    let mut out = String::with_capacity(translated.len());
    let mut last = 0;
    for (a, b) in link_spans(translated) {
        out.push_str(&translated[last..a]);
        let link = &translated[a..b];
        match src.iter().position(|l| *l == link) {
            Some(k) => out.push_str(&format!("{{{}}}", k + 1)),
            None => out.push_str(&crate::mirror::plain(link)),
        }
        last = b;
    }
    out.push_str(&translated[last..]);
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
    fn truncation_keeps_characters_whole() {
        let long = "é".repeat(200); // 400 octets
        let t = translate(&long, "DE");
        assert!(t.len() <= MAX_CHAT_BYTES);
        assert!(t.starts_with("[→DE] "));
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }
}
