//! Textes de la fenêtre de l'application dans la langue du joueur (0.16.1).
//!
//! La page elle-même (titres, boutons, jauge, légendes) est traduite par son propre dictionnaire
//! (`ui/index.html`). Les explications calculées par l'application (pourquoi un modèle tient ou
//! non, estimations, tests en jeu, notes) sont écrites en français dans le code et le journal : elles
//! sont traduites ici, phrase par phrase, d'après des modèles où `{}` marque une valeur (taille,
//! nombre, nom). Une phrase inconnue reste en français, ses mots courants traduits.
//!
//! Langues : français, anglais, allemand, espagnol, italien, russe (celles du pack low).

/// Langues proposées dans la fenêtre.
pub const LANGS: [&str; 6] = ["fr", "en", "de", "es", "it", "ru"];

/// Langue demandée par la page, ramenée à une langue connue (sinon l'anglais).
pub fn normalize(lang: &str) -> &'static str {
    let l = lang.trim().to_ascii_lowercase();
    let l = l.split(['-', '_']).next().unwrap_or("");
    LANGS.iter().find(|x| **x == l).copied().unwrap_or("en")
}

fn idx(lang: &str) -> Option<usize> {
    match lang {
        "en" => Some(0),
        "de" => Some(1),
        "es" => Some(2),
        "it" => Some(3),
        "ru" => Some(4),
        _ => None,
    }
}

/// Phrases entières (une ou plusieurs valeurs `{}`) : français, puis en, de, es, it, ru (`{0}`,
/// `{1}`... dans l'ordre des valeurs françaises).
const TEMPLATES: &[(&str, [&str; 5])] = &[
    // Pourquoi un modèle tient ou non (catalog.rs).
    (
        "ne tient pas sur cette carte : modèle {} et au moins {} pour WoW, il y a {}",
        [
            "does not fit on this card: model {0} and at least {1} for WoW, the card has {2}",
            "passt nicht auf diese Karte: Modell {0} und mindestens {1} für WoW, verfügbar sind {2}",
            "no cabe en esta tarjeta: modelo {0} y al menos {1} para WoW, hay {2}",
            "non entra in questa scheda: modello {0} e almeno {1} per WoW, ce ne sono {2}",
            "не помещается на эту видеокарту: модель {0} и не менее {1} для WoW, доступно {2}",
        ],
    ),
    (
        "{} libres pendant le jeu ; le modèle en occupe {} ({}) ; il en restera {}",
        [
            "{0} free while playing; the model takes {1} ({2}); {3} will remain",
            "{0} frei während des Spiels; das Modell belegt {1} ({2}); es bleiben {3}",
            "{0} libres durante el juego; el modelo ocupa {1} ({2}); quedarán {3}",
            "{0} liberi durante il gioco; il modello ne occupa {1} ({2}); ne resteranno {3}",
            "{0} свободно во время игры; модель занимает {1} ({2}); останется {3}",
        ],
    ),
    (
        "{} libres estimés pendant le jeu pour un modèle de {} ({}) : utilisé quand WoW laisse la place ; près de la saturation, la traduction redescend d'un cran",
        [
            "{0} estimated free while playing for a {1} model ({2}): used when WoW leaves room; near saturation, translation steps down one level",
            "{0} geschätzt frei während des Spiels für ein Modell mit {1} ({2}): genutzt, wenn WoW Platz lässt; nahe der Sättigung schaltet die Übersetzung eine Stufe zurück",
            "{0} libres estimados durante el juego para un modelo de {1} ({2}): se usa cuando WoW deja sitio; cerca de la saturación, la traducción baja un nivel",
            "{0} liberi stimati durante il gioco per un modello da {1} ({2}): usato quando WoW lascia spazio; vicino alla saturazione, la traduzione scende di un livello",
            "{0} свободно (оценка) во время игры для модели {1} ({2}): используется, когда WoW оставляет место; у предела памяти перевод опускается на ступень",
        ],
    ),
    (
        "test en jeu sur la carte graphique : {}",
        [
            "in-game test on the graphics card: {0}",
            "Test im Spiel auf der Grafikkarte: {0}",
            "prueba en el juego en la tarjeta gráfica: {0}",
            "test in gioco sulla scheda grafica: {0}",
            "проверка в игре на видеокарте: {0}",
        ],
    ),
    (
        "processeur sans AVX2 et pas de carte graphique utilisable",
        [
            "processor without AVX2 and no usable graphics card",
            "Prozessor ohne AVX2 und keine nutzbare Grafikkarte",
            "procesador sin AVX2 y ninguna tarjeta gráfica utilizable",
            "processore senza AVX2 e nessuna scheda grafica utilizzabile",
            "процессор без AVX2 и нет подходящей видеокарты",
        ],
    ),
    (
        "processeur trop juste ({} cœurs, {} de mémoire vive libres)",
        [
            "processor too weak ({0} cores, {1} of free RAM)",
            "Prozessor zu schwach ({0} Kerne, {1} freier Arbeitsspeicher)",
            "procesador insuficiente ({0} núcleos, {1} de RAM libre)",
            "processore insufficiente ({0} core, {1} di RAM libera)",
            "процессор слишком слабый ({0} ядер, свободно {1} ОЗУ)",
        ],
    ),
    (
        "sur le processeur ({} fils sur {} cœurs, plus lent qu'une carte graphique)",
        [
            "on the processor ({0} threads on {1} cores, slower than a graphics card)",
            "auf dem Prozessor ({0} Threads auf {1} Kernen, langsamer als eine Grafikkarte)",
            "en el procesador ({0} hilos en {1} núcleos, más lento que una tarjeta gráfica)",
            "sul processore ({0} thread su {1} core, più lento di una scheda grafica)",
            "на процессоре ({0} потоков на {1} ядрах, медленнее видеокарты)",
        ],
    ),
    (
        "sur le processeur : {}",
        ["on the processor: {0}", "auf dem Prozessor: {0}", "en el procesador: {0}", "sul processore: {0}", "на процессоре: {0}"],
    ),
    // Estimations (plan.rs).
    (
        "mémoire vidéo : {} pour le modèle et au moins {} pour WoW, la carte en a {}",
        [
            "video memory: {0} for the model and at least {1} for WoW, the card has {2}",
            "Grafikspeicher: {0} für das Modell und mindestens {1} für WoW, die Karte hat {2}",
            "memoria de vídeo: {0} para el modelo y al menos {1} para WoW, la tarjeta tiene {2}",
            "memoria video: {0} per il modello e almeno {1} per WoW, la scheda ne ha {2}",
            "видеопамять: {0} для модели и не менее {1} для WoW, у видеокарты {2}",
        ],
    ),
    (
        "interdit après un échec : {}",
        ["ruled out after a failure: {0}", "nach einem Fehlschlag ausgeschlossen: {0}", "descartado tras un fallo: {0}", "escluso dopo un errore: {0}", "исключено после сбоя: {0}"],
    ),
    (
        "{} libres estimés pendant le jeu : pris quand la place est là, redescend avant la saturation",
        [
            "{0} estimated free while playing: used when there is room, steps down before saturation",
            "{0} geschätzt frei während des Spiels: genutzt, wenn Platz ist, schaltet vor der Sättigung zurück",
            "{0} libres estimados durante el juego: se usa cuando hay sitio, baja antes de la saturación",
            "{0} liberi stimati durante il gioco: usato quando c'è spazio, scende prima della saturazione",
            "{0} свободно (оценка) во время игры: используется, когда есть место, опускается до предела памяти",
        ],
    ),
    (
        "{} cœurs, il en faut {}",
        ["{0} cores, {1} needed", "{0} Kerne, {1} nötig", "{0} núcleos, se necesitan {1}", "{0} core, ne servono {1}", "{0} ядер, нужно {1}"],
    ),
    (
        "mémoire vive : {} nécessaires, {} libres",
        ["RAM: {0} needed, {1} free", "Arbeitsspeicher: {0} nötig, {1} frei", "RAM: {0} necesarios, {1} libres", "RAM: {0} necessari, {1} liberi", "ОЗУ: нужно {0}, свободно {1}"],
    ),
    (
        "test en jeu : {}",
        ["in-game test: {0}", "Test im Spiel: {0}", "prueba en el juego: {0}", "test in gioco: {0}", "проверка в игре: {0}"],
    ),
    (
        "{} s par message, jusqu'à {} messages par minute sous 5 % de baisse",
        [
            "{0} s per message, up to {1} messages per minute under a 5% drop",
            "{0} s pro Nachricht, bis zu {1} Nachrichten pro Minute bei unter 5 % Einbruch",
            "{0} s por mensaje, hasta {1} mensajes por minuto con menos de un 5 % de caída",
            "{0} s per messaggio, fino a {1} messaggi al minuto sotto il 5 % di calo",
            "{0} с на сообщение, до {1} сообщений в минуту при падении менее 5 %",
        ],
    ),
    (
        "{} s par message (seuil {} s)",
        ["{0} s per message (limit {1} s)", "{0} s pro Nachricht (Grenze {1} s)", "{0} s por mensaje (límite {1} s)", "{0} s per messaggio (soglia {1} s)", "{0} с на сообщение (порог {1} с)"],
    ),
    (
        "{} messages par minute sous 5 % de baisse (il en faut {})",
        [
            "{0} messages per minute under a 5% drop ({1} needed)",
            "{0} Nachrichten pro Minute bei unter 5 % Einbruch ({1} nötig)",
            "{0} mensajes por minuto con menos de un 5 % de caída (se necesitan {1})",
            "{0} messaggi al minuto sotto il 5 % di calo (ne servono {1})",
            "{0} сообщений в минуту при падении менее 5 % (нужно {1})",
        ],
    ),
    (
        "prévision : {}",
        ["forecast: {0}", "Prognose: {0}", "previsión: {0}", "previsione: {0}", "прогноз: {0}"],
    ),
    (
        "test en jeu du {}",
        ["in-game test of {0}", "Test im Spiel vom {0}", "prueba en el juego del {0}", "test in gioco del {0}", "проверка в игре от {0}"],
    ),
    (
        "mesuré le {}",
        ["measured on {0}", "gemessen am {0}", "medido el {0}", "misurato il {0}", "измерено {0}"],
    ),
    (
        "pilote réinitialisé {} fois",
        ["driver reset {0} times", "Treiber {0}-mal zurückgesetzt", "controlador reiniciado {0} veces", "driver reimpostato {0} volte", "драйвер перезапущен {0} раз"],
    ),
    // Notes (catalog.rs).
    (
        "{} de mémoire vidéo comptés pour WoW (valeur de référence, raids compris) ; le test en jeu confirmera.",
        [
            "{0} of video memory counted for WoW (reference value, raids included); the in-game test will confirm it.",
            "{0} Grafikspeicher für WoW angesetzt (Referenzwert, Schlachtzüge inklusive); der Test im Spiel bestätigt es.",
            "{0} de memoria de vídeo contados para WoW (valor de referencia, bandas incluidas); la prueba en el juego lo confirmará.",
            "{0} di memoria video contati per WoW (valore di riferimento, incursioni comprese); il test in gioco lo confermerà.",
            "{0} видеопамяти отведено для WoW (опорное значение, рейды включены); проверка в игре это подтвердит.",
        ],
    ),
    (
        "Ce PC ne peut pas faire tourner la traduction locale (processeur sans AVX2 ou mémoire insuffisante).",
        [
            "This PC cannot run local translation (processor without AVX2 or not enough memory).",
            "Dieser PC kann die lokale Übersetzung nicht ausführen (Prozessor ohne AVX2 oder zu wenig Speicher).",
            "Este PC no puede ejecutar la traducción local (procesador sin AVX2 o memoria insuficiente).",
            "Questo PC non può eseguire la traduzione locale (processore senza AVX2 o memoria insufficiente).",
            "Этот ПК не может выполнять локальный перевод (процессор без AVX2 или мало памяти).",
        ],
    ),
    (
        "Seul le pack low est possible : les messages dans les autres langues resteront affichés sans traduction.",
        [
            "Only the low pack is possible: messages in other languages will stay untranslated.",
            "Nur das Low-Paket ist möglich: Nachrichten in anderen Sprachen bleiben unübersetzt.",
            "Solo es posible el pack low: los mensajes en otros idiomas se mostrarán sin traducir.",
            "È possibile solo il pacchetto low: i messaggi nelle altre lingue resteranno senza traduzione.",
            "Возможен только пакет low: сообщения на других языках останутся без перевода.",
        ],
    ),
];

/// Mots et morceaux de phrase courants, remplacés dans ce qui ne correspond à aucun modèle (et dans
/// les valeurs des modèles). Les plus longs d'abord.
const PHRASES: &[(&str, [&str; 5])] = &[
    ("pire cas (WoW occupe toute la carte)", ["worst case (WoW uses the whole card)", "ungünstigster Fall (WoW belegt die ganze Karte)", "peor caso (WoW ocupa toda la tarjeta)", "caso peggiore (WoW occupa tutta la scheda)", "худший случай (WoW занимает всю видеокарту)"]),
    ("mesurées au test en jeu du", ["measured in the in-game test of", "gemessen beim Test im Spiel vom", "medidas en la prueba en el juego del", "misurate nel test in gioco del", "измерено при проверке в игре"]),
    ("pas de carte graphique utilisable", ["no usable graphics card", "keine nutzbare Grafikkarte", "ninguna tarjeta gráfica utilizable", "nessuna scheda grafica utilizzabile", "нет подходящей видеокарты"]),
    ("mémoire vidéo insuffisante", ["not enough video memory", "zu wenig Grafikspeicher", "memoria de vídeo insuficiente", "memoria video insufficiente", "недостаточно видеопамяти"]),
    ("trop lent sur le processeur", ["too slow on the processor", "zu langsam auf dem Prozessor", "demasiado lento en el procesador", "troppo lento sul processore", "слишком медленно на процессоре"]),
    ("a fait planter le jeu", ["crashed the game", "hat das Spiel abstürzen lassen", "hizo fallar el juego", "ha fatto crashare il gioco", "привело к сбою игры"]),
    ("sur la carte graphique", ["on the graphics card", "auf der Grafikkarte", "en la tarjeta gráfica", "sulla scheda grafica", "на видеокарте"]),
    ("processeur sans AVX2", ["processor without AVX2", "Prozessor ohne AVX2", "procesador sin AVX2", "processore senza AVX2", "процессор без AVX2"]),
    ("sur le processeur", ["on the processor", "auf dem Prozessor", "en el procesador", "sul processore", "на процессоре"]),
    ("mesuré sur ce PC", ["measured on this PC", "auf diesem PC gemessen", "medido en este PC", "misurato su questo PC", "измерено на этом ПК"]),
    ("échelle de rendu", ["render scale", "Renderskalierung", "escala de renderizado", "scala di rendering", "масштаб отрисовки"]),
    ("réglages de WoW", ["WoW settings", "WoW-Einstellungen", "ajustes de WoW", "impostazioni di WoW", "настройки WoW"]),
    ("sans traduction", ["without translation", "ohne Übersetzung", "sin traducción", "senza traduzione", "без перевода"]),
    ("non plafonné", ["uncapped", "unbegrenzt", "sin límite", "senza limite", "без ограничения"]),
    ("plafonné à", ["capped at", "begrenzt auf", "limitado a", "limitato a", "ограничено"]),
    ("trop lourd", ["too heavy", "zu schwer", "demasiado pesado", "troppo pesante", "слишком тяжёлая"]),
    ("estimation", ["estimate", "Schätzung", "estimación", "stima", "оценка"]),
    ("prévision", ["forecast", "Prognose", "previsión", "previsione", "прогноз"]),
    ("images/s", ["FPS", "FPS", "FPS", "FPS", "кадр/с"]),
    ("qualité", ["quality", "Qualität", "calidad", "qualità", "качество"]),
    ("réussi", ["passed", "bestanden", "superada", "superato", "успешно"]),
    ("version", ["version", "Version", "versión", "versione", "версия"]),
];

/// Correspondance d'un texte avec un modèle (`{}` : une valeur) : les valeurs trouvées.
fn capture(tpl: &str, text: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = tpl.split("{}").collect();
    let first = parts[0];
    if !text.starts_with(first) {
        return None;
    }
    let mut pos = first.len();
    let mut caps = Vec::new();
    let n = parts.len();
    for (i, lit) in parts.iter().enumerate().skip(1) {
        let last = i == n - 1;
        let end = if last {
            if lit.is_empty() {
                text.len()
            } else if text.len() >= pos + lit.len() && text.ends_with(lit) {
                text.len() - lit.len()
            } else {
                return None;
            }
        } else if lit.is_empty() {
            return None;
        } else {
            pos + text[pos..].find(lit)?
        };
        if end < pos {
            return None;
        }
        caps.push(text[pos..end].to_string());
        pos = end + lit.len();
    }
    (pos == text.len()).then_some(caps)
}

fn fill(tpl: &str, vals: &[String]) -> String {
    let mut out = tpl.to_string();
    for (i, v) in vals.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), v);
    }
    out
}

/// Unités et nombres : « 3,4 Go » -> « 3.4 GB » (anglais), « 3,4 GB », « 3,4 ГБ ».
fn units(lang: &str, s: &str) -> String {
    let unit = if lang == "ru" { "ГБ" } else { "GB" };
    let s = s.replace(" Go", &format!(" {unit}")).replace("\u{a0}Go", &format!("\u{a0}{unit}"));
    if lang != "en" {
        return s;
    }
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, ch) in c.iter().enumerate() {
        let digit = |j: Option<&char>| j.is_some_and(|x| x.is_ascii_digit());
        if *ch == ',' && i > 0 && digit(c.get(i - 1)) && digit(c.get(i + 1)) {
            out.push('.');
        } else {
            out.push(*ch);
        }
    }
    out.replace(" %", "%")
}

/// Morceaux entre parenthèses qui correspondent à un modèle : traduits.
fn parens(lang: &str, k: usize, s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('(') {
        let Some(b) = rest[a..].find(')').map(|b| a + b) else { break };
        out.push_str(&rest[..=a]);
        let inner = &rest[a + 1..b];
        out.push_str(&one(lang, k, inner).unwrap_or_else(|| inner.to_string()));
        out.push(')');
        rest = &rest[b + 1..];
    }
    out.push_str(rest);
    out
}

fn phrases(k: usize, s: &str) -> String {
    let mut out = s.to_string();
    for (fr, t) in PHRASES {
        if out.contains(fr) {
            out = out.replace(fr, t[k]);
        }
    }
    out
}

fn one(lang: &str, k: usize, text: &str) -> Option<String> {
    for (fr, t) in TEMPLATES {
        if let Some(vals) = capture(fr, text) {
            // Une valeur en tête de phrase qui contient elle-même plusieurs morceaux (« …, … ») :
            // ce n'est pas ce modèle, c'est une liste (traitée morceau par morceau).
            if vals.iter().any(|v| v.contains(" ; ")) || (fr.starts_with("{}") && vals[0].contains(", ")) {
                continue;
            }
            let vals: Vec<String> = vals.iter().map(|v| translate(lang, v)).collect();
            return Some(fill(t[k], &vals));
        }
    }
    None
}

/// Texte de l'application dans la langue de la page (`fr` : tel quel).
pub fn translate(lang: &str, text: &str) -> String {
    let lang = normalize(lang);
    let Some(k) = idx(lang) else { return text.to_string() };
    if text.trim().is_empty() {
        return text.to_string();
    }
    let out = one(lang, k, text)
        .or_else(|| {
            // Plusieurs explications mises bout à bout.
            if !text.contains(" ; ") {
                return None;
            }
            Some(text.split(" ; ").map(|p| one(lang, k, p).unwrap_or_else(|| phrases(k, &parens(lang, k, p)))).collect::<Vec<_>>().join("; "))
        })
        .or_else(|| {
            // « … (seuil …), … messages par minute … » : chaque morceau doit être connu.
            let parts: Vec<&str> = text.split(", ").collect();
            if parts.len() < 2 {
                return None;
            }
            let done: Vec<Option<String>> = parts.iter().map(|p| one(lang, k, p)).collect();
            done.iter().all(Option::is_some).then(|| done.into_iter().flatten().collect::<Vec<_>>().join(", "))
        })
        .unwrap_or_else(|| phrases(k, &parens(lang, k, text)));
    units(lang, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explanations_follow_the_page_language() {
        assert_eq!(normalize("de-DE"), "de");
        assert_eq!(normalize("pt-BR"), "en");
        assert_eq!(normalize("FR"), "fr");
        let r = "8,7 Go libres pendant le jeu ; le modèle en occupe 5,3 Go (mesuré sur ce PC) ; il en restera 3,4 Go";
        assert_eq!(translate("fr", r), r);
        assert_eq!(translate("en", r), "8.7 GB free while playing; the model takes 5.3 GB (measured on this PC); 3.4 GB will remain");
        assert_eq!(translate("de", r), "8,7 GB frei während des Spiels; das Modell belegt 5,3 GB (auf diesem PC gemessen); es bleiben 3,4 GB");
        assert_eq!(translate("ru", r), "8,7 ГБ свободно во время игры; модель занимает 5,3 ГБ (измерено на этом ПК); останется 3,4 ГБ");
        // Deux explications bout à bout, la seconde inconnue : ses mots courants traduits.
        let r2 = "test en jeu sur la carte graphique : a fait planter le jeu (pilote réinitialisé 3 fois) ; sur le processeur : trop lourd";
        assert_eq!(
            translate("es", r2),
            "prueba en el juego en la tarjeta gráfica: hizo fallar el juego (controlador reiniciado 3 veces); en el procesador: demasiado pesado"
        );
        assert_eq!(
            translate("en", "0,62 s par message (seuil 1 s), 12 messages par minute sous 5 % de baisse (il en faut 20)"),
            "0.62 s per message (limit 1 s), 12 messages per minute under a 5% drop (20 needed)"
        );
        assert_eq!(translate("it", "prévision : mesuré le 09/11/2026"), "previsione: misurato il 09/11/2026");
        assert_eq!(
            translate("en", "version 12.1.5, 3840x1600 (échelle de rendu 77 %), qualité 7/10, non plafonné - réglages de WoW (_retail_\\WTF\\Config.wtf)"),
            "version 12.1.5, 3840x1600 (render scale 77%), quality 7/10, uncapped - WoW settings (_retail_\\WTF\\Config.wtf)"
        );
        // Phrase inconnue : laissée telle quelle (mots connus traduits).
        assert_eq!(translate("en", "quelque chose de nouveau"), "quelque chose de nouveau");
        assert!(TEMPLATES.iter().all(|(fr, t)| {
            let n = fr.matches("{}").count();
            t.iter().all(|x| (0..n).all(|i| x.contains(&format!("{{{i}}}"))))
        }), "chaque traduction reprend toutes les valeurs");
    }
}
