//! Ce que le modèle de traduction ne doit pas toucher : liens du jeu (objets, sorts, quêtes),
//! icônes, marques de raid, noms propres du jeu et jargon. Avant la traduction, chacun est
//! remplacé par un repère « [1] », « [2] »… que les modèles recopient ; après, le repère est
//! remplacé par l'original (lien, mot gardé), par le nom dans la langue du lecteur (donjons,
//! raids, lieux) ou par la traduction du dictionnaire (acronymes et jargon du jeu, 0.12.4) :
//! `data/termes.tsv`.

use std::sync::OnceLock;

const TERMS: &str = include_str!("../data/termes.tsv");

pub struct Entry {
    /// Écritures par langue (la première est celle qu'on écrit).
    names: Vec<(String, Vec<String>)>,
}

impl Entry {
    fn names_in(&self, lang: &str) -> Option<&Vec<String>> {
        self.names.iter().find(|(l, v)| l == lang && !v.is_empty()).map(|(_, v)| v)
    }
}

pub struct Glossary {
    pub entries: Vec<Entry>,
    /// Acronymes et jargon du jeu, traduits par le dictionnaire (« WTS » : « Vends »).
    pub jargon: Vec<Entry>,
    pub keep: Vec<String>,
}

fn parse_row(header: &[String], l: &str) -> Entry {
    let names = header
        .iter()
        .zip(l.split('\t').chain(std::iter::repeat("")))
        .map(|(lang, cell)| (lang.clone(), cell.split('|').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()))
        .collect();
    Entry { names }
}

pub fn parse(text: &str) -> Glossary {
    let mut g = Glossary { entries: Vec::new(), jargon: Vec::new(), keep: Vec::new() };
    let mut section = "";
    let mut header: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim_end_matches('\r');
        if l.starts_with('#') || l.trim().is_empty() {
            continue;
        }
        if l.starts_with('[') {
            section = if l.starts_with("[noms]") {
                "noms"
            } else if l.starts_with("[jargon]") {
                "jargon"
            } else if l.starts_with("[garder]") {
                "garder"
            } else {
                ""
            };
            header.clear();
            continue;
        }
        match section {
            "noms" | "jargon" if header.is_empty() => header = l.split('\t').map(|s| s.trim().to_string()).collect(),
            "noms" => g.entries.push(parse_row(&header, l)),
            "jargon" => g.jargon.push(parse_row(&header, l)),
            "garder" => g.keep.extend(l.split_whitespace().map(|w| w.to_lowercase())),
            _ => {}
        }
    }
    g
}

pub fn glossary() -> &'static Glossary {
    static G: OnceLock<Glossary> = OnceLock::new();
    G.get_or_init(|| parse(TERMS))
}

/// Texte prêt à traduire, et ce que chaque repère redevient.
#[derive(Debug, Default, PartialEq)]
pub struct Protected {
    pub text: String,
    pub slots: Vec<Slot>,
}

#[derive(Debug, PartialEq)]
pub struct Slot {
    pub restore: String,
    /// Un lien ou une icône : à remettre même si le modèle a perdu son repère.
    pub essential: bool,
    /// Mot du dictionnaire (« Vends ») : majuscule en début de phrase seulement.
    pub fit_case: bool,
}

/// Acronyme écrit en capitales (« MP », « DD », « HV ») : reconnu aussi dans la langue du message.
fn is_acronym(w: &str) -> bool {
    w.chars().count() <= 5 && w.chars().any(|c| c.is_alphabetic()) && !w.chars().any(|c| c.is_lowercase())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\''
}

/// Forme du dictionnaire telle qu'elle s'écrit (sans le « = » des formes à casse exacte).
fn written(form: &str) -> &str {
    form.strip_prefix('=').unwrap_or(form)
}

/// Position d'une forme du dictionnaire : casse indifférente, sauf « =FORME » (casse exacte).
fn find_form(hay: &str, form: &str, from: usize) -> Option<usize> {
    let Some(exact) = form.strip_prefix('=') else { return find_phrase(hay, form, from) };
    let mut start = from;
    while let Some(i) = hay.get(start..)?.find(exact) {
        let at = start + i;
        let before = hay[..at].chars().next_back();
        let after = hay[at + exact.len()..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return Some(at);
        }
        start = at + exact.len().max(1);
    }
    None
}

/// Position d'un mot ou d'une expression entière, sans tenir compte de la casse.
fn find_phrase(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.to_lowercase();
    let n = needle.to_lowercase();
    // La casse peut changer la longueur en octets (rare) : on ne cherche que si elle est stable.
    if h.len() != hay.len() || n.is_empty() {
        return None;
    }
    let mut start = from;
    while let Some(i) = h[start..].find(&n) {
        let at = start + i;
        let before = hay[..at].chars().next_back();
        let after = hay[at + n.len()..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return Some(at);
        }
        start = at + n.len().max(1);
        if start >= h.len() {
            break;
        }
    }
    None
}

/// Fin d'un lien commençant à `i` (« |H…|h[…]|h », précédé de « |cAARRGGBB », suivi de « |r »).
fn link_end(s: &str, i: usize) -> Option<usize> {
    let rest = &s[i..];
    let h = if rest.starts_with("|c") {
        rest.find("|H")?
    } else if rest.starts_with("|H") {
        0
    } else {
        return None;
    };
    let data_end = h + 2 + rest[h + 2..].find("|h")?;
    let text_end = data_end + 2 + rest[data_end + 2..].find("|h")? + 2;
    let end = if rest[text_end..].starts_with("|r") { text_end + 2 } else { text_end };
    Some(i + end)
}

pub fn protect(text: &str, src: &str, tgt: &str) -> Protected {
    let g = glossary();
    let mut p = Protected::default();
    let mut out = String::new();
    // 1. Liens, icônes |T…|t et |A…|a, marques de raid {rt1} {étoile}.
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let end = if rest.starts_with("|c") || rest.starts_with("|H") {
            link_end(text, i)
        } else if rest.starts_with("|T") {
            rest.find("|t").map(|e| i + e + 2)
        } else if rest.starts_with("|A") {
            rest.find("|a").map(|e| i + e + 2)
        } else if rest.starts_with('{') {
            rest.find('}').filter(|e| *e <= 20).map(|e| i + e + 1)
        } else {
            None
        };
        match end {
            Some(e) => {
                p.slots.push(Slot { restore: text[i..e].to_string(), essential: true, fit_case: false });
                out.push_str(&format!("[{}]", p.slots.len()));
                i = e;
            }
            None => {
                let c = rest.chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    // 2. Noms propres du jeu (langue du message, ou anglais), les plus longs d'abord.
    let mut names: Vec<(&str, &Entry)> = Vec::new();
    for e in &g.entries {
        for lang in [src, "en"] {
            if let Some(v) = e.names_in(lang) {
                names.extend(v.iter().map(|n| (n.as_str(), e)));
            }
        }
    }
    names.sort_by_key(|(n, _)| std::cmp::Reverse(written(n).len()));
    for (name, e) in names {
        let mut from = 0;
        let len = written(name).len();
        while let Some(at) = find_form(&out, name, from) {
            let original = out[at..at + len].to_string();
            let shown = e.names_in(tgt).map_or(original, |v| written(&v[0]).to_string());
            p.slots.push(Slot { restore: shown, essential: false, fit_case: true });
            let mark = format!("[{}]", p.slots.len());
            out.replace_range(at..at + len, &mark);
            from = at + mark.len();
        }
    }
    // 3. Acronymes et jargon du jeu : traduits par le dictionnaire (formes anglaises, et acronymes
    // de la langue du message), les plus longs d'abord. Case vide dans la langue du lecteur : gardé.
    let mut words: Vec<(&str, &Entry)> = Vec::new();
    for e in &g.jargon {
        if let Some(v) = e.names_in("en") {
            words.extend(v.iter().map(|n| (n.as_str(), e)));
        }
        if src != "en" {
            if let Some(v) = e.names_in(src) {
                words.extend(v.iter().filter(|n| is_acronym(written(n))).map(|n| (n.as_str(), e)));
            }
        }
    }
    words.sort_by_key(|(n, _)| std::cmp::Reverse(written(n).len()));
    for (word, e) in words {
        let mut from = 0;
        let len = written(word).len();
        while let Some(at) = find_form(&out, word, from) {
            let original = out[at..at + len].to_string();
            let shown = e.names_in(tgt).map_or(original, |v| written(&v[0]).to_string());
            p.slots.push(Slot { restore: shown, essential: false, fit_case: true });
            let mark = format!("[{}]", p.slots.len());
            out.replace_range(at..at + len, &mark);
            from = at + mark.len();
        }
    }
    // 4. Mots gardés tels quels.
    for w in &g.keep {
        let mut from = 0;
        while let Some(at) = find_phrase(&out, w, from) {
            let original = out[at..at + w.len()].to_string();
            p.slots.push(Slot { restore: original, essential: false, fit_case: false });
            let mark = format!("[{}]", p.slots.len());
            out.replace_range(at..at + w.len(), &mark);
            from = at + mark.len();
        }
    }
    p.text = out;
    p
}

/// Articles en tête d'un nom (« Les Mortemines ») : en minuscule au milieu d'une phrase.
const ARTICLES: &[&str] = &[
    "le", "la", "les", "l", "the", "der", "die", "das", "el", "los", "las", "il", "lo", "gli", "i", "o", "os", "as",
];

/// Mot du dictionnaire placé après `before` : majuscule en début de phrase (« Vends »), tel
/// quel ailleurs (« vends », « Tank » en allemand) ; article d'un nom en minuscule au milieu
/// d'une phrase (« dans les Mortemines »).
fn fit_case(word: &str, before: &str) -> String {
    let start = before.trim_end().chars().next_back().is_none_or(|c| matches!(c, '.' | '!' | '?' | ':' | '¡' | '¿'));
    let mut cs = word.chars();
    let Some(first) = cs.next() else { return String::new() };
    if start {
        return first.to_uppercase().chain(cs).collect();
    }
    let head = word.split([' ', '\'', '’']).next().unwrap_or("");
    if word.contains([' ', '\'', '’']) && first.is_uppercase() && ARTICLES.contains(&head.to_lowercase().as_str()) {
        return first.to_lowercase().chain(cs).collect();
    }
    word.to_string()
}

/// Remet les liens, noms et jargon à la place de leurs repères. Un lien dont le modèle a perdu
/// le repère est remis en fin de message (on ne perd jamais un objet).
pub fn restore(translated: &str, p: &Protected) -> String {
    let mut s = translated.to_string();
    let mut lost = Vec::new();
    // Du plus grand numéro au plus petit : « [12] » avant « [1] ».
    for (k, slot) in p.slots.iter().enumerate().rev() {
        let mark = format!("[{}]", k + 1);
        if let Some(at) = s.find(&mark) {
            let shown = if slot.fit_case { fit_case(&slot.restore, &s[..at]) } else { slot.restore.clone() };
            s.replace_range(at..at + mark.len(), &shown);
            s = s.replace(&mark, "");
        } else if slot.essential {
            lost.push(slot.restore.clone());
        }
    }
    for l in lost.into_iter().rev() {
        s.push(' ');
        s.push_str(&l);
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "|cffa335ee|Hitem:19019::::::::60:::::|h[Thunderfury, Blessed Blade of the Windseeker]|h|r";

    #[test]
    fn dictionary_file_reads() {
        let g = glossary();
        assert!(g.entries.len() >= 60);
        assert!(g.jargon.len() >= 30);
        assert!(g.keep.contains(&"craft".to_string()));
        let mc = &g.entries[0];
        assert_eq!(mc.names_in("fr").unwrap()[0], "Cœur du Magma");
        assert_eq!(mc.names_in("en").unwrap(), &vec!["Molten Core".to_string(), "=MC".to_string()]);
        // Case vide : pas de nom dans cette langue.
        assert!(g.entries[1].names_in("it").is_none());
    }

    #[test]
    fn links_names_and_jargon_survive_translation() {
        let p = protect(&format!("WTS {LINK}, whisper me offers"), "en", "fr");
        assert_eq!(p.text, "[3] [1], [2] offers");
        // Le modèle traduit autour des repères.
        assert_eq!(restore("[3] [1], [2] vos offres", &p), format!("Vends {LINK}, MP vos offres"));
        // Repère perdu : le lien est remis à la fin.
        assert_eq!(restore("[3], [2] vos offres", &p), format!("Vends, MP vos offres {LINK}"));
        // Nom propre dans la langue du message ou en anglais : nom du lecteur ; acronymes et
        // jargon traduits par le dictionnaire (0.12.4).
        let p = protect("LFM Molten Core, need 1 tank and 2 healers, PST", "en", "de");
        assert_eq!(p.text, "[4] [1], need 1 [3] and 2 [2], [5]");
        assert_eq!(restore("[4] [1], brauche 1 [3] und 2 [2], [5]", &p), "Suche Mitspieler Geschmolzener Kern, brauche 1 Tank und 2 Heiler, /w");
        let p = protect("LF tank et heal pour Cœur du Magma, on part dans 10 min", "fr", "es");
        assert_eq!(p.text, "[4] [2] et [3] pour [1], on part dans 10 min");
        assert_eq!(restore("[4] [2] y [3] para [1], salimos en 10 min", &p), "Busco tanque y healer para Núcleo de Magma, salimos en 10 min");
        // Vers le français : « WTS » devient « Vends », en minuscule au milieu d'une phrase.
        let p = protect("hi, wts boost M+ tonight in Windrunner Spire, pst", "en", "fr");
        assert_eq!(p.text, "hi, [2] boost [4] tonight in [1], [3]");
        assert_eq!(restore("salut, [2] boost [4] ce soir dans [1], [3]", &p), "salut, vends boost MM+ ce soir dans Flèche de Coursevent, MP");
        assert_eq!(restore("[2] boost [4] ce soir", &p), "Vends boost MM+ ce soir");
        // Article d'un nom au milieu d'une phrase.
        let p = protect("anyone for Deadmines?", "en", "fr");
        assert_eq!(restore("quelqu'un pour [1] ?", &p), "quelqu'un pour les Mortemines ?");
        assert_eq!(restore("[1] ?", &p), "Les Mortemines ?");
        // Formes à casse exacte : « if » et « ah » restent des mots ; « IF » et « AH » sont du jeu.
        assert_eq!(protect("ah ok, if you want", "en", "fr").text, "ah ok, if you want");
        let p = protect("meet at AH in IF", "en", "fr");
        assert_eq!(restore(&p.text, &p), "meet at HV in Forgefer");
        // Acronyme en capitales de la langue du message : « MP » vers l'anglais.
        let p = protect("MP moi pour le craft", "fr", "en");
        assert_eq!(restore("[1] me for the [2]", &p), "PST me for the craft");
        // Nom inconnu dans la langue du lecteur : l'original reste.
        let p = protect("Quelqu'un pour Repaire d'Onyxia ?", "fr", "it");
        assert_eq!(restore("Qualcuno per [1]?", &p), "Qualcuno per Repaire d'Onyxia?");
        // Mots entiers seulement : « tanker » ou « Mortemines » dans un mot plus long ne bougent pas.
        assert_eq!(protect("je vais tanker", "fr", "en").text, "je vais tanker");
        // Icônes et marques de raid.
        let p = protect("pull sur {rt8} |TInterface\\Icons\\x:0|t go", "fr", "en");
        assert_eq!(p.text, "pull sur [1] [2] go");
    }
}
