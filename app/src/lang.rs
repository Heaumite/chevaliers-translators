//! Détection de la langue d'un message de chat, pour les 23 langues du projet.
//!
//! Les messages de chat sont courts et pleins de jargon : un détecteur général (`whatlang`,
//! fréquences de trigrammes) se trompe sur les langues proches (norvégien et danois, russe,
//! ukrainien et bulgare, italien et espagnol). On ajoute ce qui tranche dans un message court :
//!   - l'écriture (grec, cyrillique) et les lettres propres à une langue (ș ț roumain, ő ű
//!     hongrois, ř ů ě tchèque, ł ą ę polonais, ы э ё russe, і ї є ukrainien, ђ ћ љ serbe…) ;
//!   - les mots courants du chat dans chaque langue (salut, danke, gracias, takk, tak…) ;
//!   - le jargon du jeu (LF, WTS, tank, heal, gg…) ne compte pour aucune langue : un message
//!     qui n'a que du jargon est « neutre » et n'est pas traduit.
//! La mémoire de la langue de chaque joueur (`Speakers`) départage les messages incertains.

use std::collections::{HashMap, VecDeque};

/// Les 23 langues du projet (codes ISO 639-1), dans l'ordre du banc.
pub const LANGS: &[&str] = &[
    "fr", "en", "de", "es", "it", "ru", "pt", "nl", "pl", "cs", "sk", "hu", "ro", "bg", "el", "hr", "sr", "sl", "uk", "sv",
    "no", "fi", "da",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Detected {
    /// Langue reconnue, avec un indice de confiance de 0 à 1.
    Lang(&'static str, f32),
    /// Rien qui permette de trancher (jargon, nombres, liens, une ou deux lettres).
    Neutral,
}

impl Detected {
    #[allow(dead_code)]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Detected::Lang(c, _) => Some(c),
            Detected::Neutral => None,
        }
    }
    #[allow(dead_code)]
    pub fn confidence(&self) -> f32 {
        match self {
            Detected::Lang(_, c) => *c,
            Detected::Neutral => 0.0,
        }
    }
}

/// Jargon du jeu compris par tous : il ne désigne aucune langue (et n'est jamais traduit, voir
/// `glossary`). En minuscules.
pub const JARGON: &[&str] = &[
    "lf", "lfm", "lfg", "lf1m", "lf2m", "lf3m", "lf4m", "wts", "wtb", "wtt", "pst", "dps", "tank", "tanks", "heal", "heals",
    "healer", "healers", "heals", "inv", "invit", "pls", "plz", "ty", "thx", "gz", "grats", "gg", "lol", "brb", "afk", "rdy",
    "ok", "okay", "kk", "np", "ms", "os", "sr", "hc", "aoe", "cc", "pug", "dkp", "gdkp", "mt", "ot", "ml", "kek", "xd", "omg",
    "wtf", "hf", "gl", "hp", "mana", "buff", "debuff", "loot", "boost", "summ", "sum", "port", "res", "rez", "mc", "bwl", "zg",
    "aq", "aq20", "aq40", "ony", "naxx", "brd", "lbrs", "ubrs", "strat", "scholo", "st", "zf", "mara", "rfc", "wc", "vc", "dm",
    "sfk", "bfd", "rfk", "sm", "rfd", "ulda", "gnomer", "lvl", "lv", "xp", "exp", "quest", "raid", "raids", "dungeon", "dung",
    "dd", "rogue", "mage", "priest", "warlock", "lock", "hunter", "druid", "dudu", "pala", "paladin", "shaman", "chamy", "war",
    "warri", "warrior", "craft", "crafts", "recraft", "crafter", "g", "s", "c", "k",
];

/// Mots courants du chat, par langue (minuscules). Un mot partagé compte pour chaque langue.
const WORDS: &[(&str, &str)] = &[
    ("fr", "le la les un une des de du et est je tu il nous vous on pour avec dans sur pas que qui quoi quelqu salut merci bonjour bonsoir oui non mais ou donc cette ce ces suis es sont fait faire peut besoin encore aussi très bien moi toi cherche reviens attendez soir soirée svp stp faut groupe donjon quête tout monde aider minutes min mange vends vend achète achete prix po mp moi bon ok reviens vais mais comme alors très vraiment beaucoup mieux que pense sais ouais pourquoi comment où quand tous trop aussi parce rien personne quelque chose toujours jamais peut-être bien sûr zone gens truc ça cela plus moins plaît aime"),
    ("en", "the a an and is are you i we they he she it for with in on to of that this what who anyone someone thanks thank please need still more can help me my have has will want looking back wait good night hey hi hello yes no not just one up run everyone evening group minutes was that had left close selling buying sell buy price whisper offers gold i'm im its dont can't cant got get go going there here from but like so very really much better then than think know yeah yep nope why how where when all too also because about would could should been were did does doing done make made zone people nobody somebody everybody something anything nothing always never maybe probably actually lot little bit way much many other another same still only even well right sure guess mean feel seems look looks pretty fine nice cool fun hate love enjoy boy guys dude mate bro folks sorry welcome bye morning today tonight tomorrow yesterday again ready come join anybody everything friend friends talk speak english"),
    ("de", "der die das ein eine einen und ist bin sind ich du wir ihr er sie es für mit auf zu von dem den nicht noch auch jemand hallo danke bitte kann mir mich mal gruppe suche brauchen wartet zurück abend schönen hat lust zusammen bei dieser helfen minuten verkaufe kaufe preis gold flüstert flüster wer was wie gleich kurz bin weg essen aber wie also sehr wirklich viel besser als denke weiß ja warum wo wann alle zu auch weil nichts niemand etwas immer nie vielleicht gut sicher leute mag gerne"),
    ("es", "el la los las un una unos y es soy son yo tú nosotros para con en por de del que quién alguien hola gracias buenas buenos noches sí no pero también todavía necesitamos busco puede ayudar vuelvo esperadme favor grupo hermandad todos noche me esta vendo compro precio oro privado susurra susurrad voy ahora luego ya cuidado pero como entonces muy realmente mucho mejor que pienso sé sí porque dónde cuándo todos también nada nadie algo siempre nunca quizás bien seguro gente gusta"),
    ("it", "il lo la gli le un una e è sono io tu noi per con in su di del che chi qualcuno ciao grazie buona buongiorno sera serata sì no ma anche ancora serve cerco può aiutare torno aspettatemi favore gruppo gilda tutti mi questa tra vendo compro prezzo oro privato scrivetemi scrivete vado adesso subito dopo attenzione ma come allora molto davvero meglio che penso so sì perché dove quando tutti anche niente nessuno qualcosa sempre mai forse bene sicuro gente piace"),
    ("pt", "o a os as um uma e é sou são eu tu nós para com em no na de do da que quem alguém olá obrigado obrigada boa noite sim não mas também ainda precisamos procuro pode ajudar volto esperem favor grupo todos me esta vendo compro preço ouro privado sussurra vou agora depois cuidado"),
    ("nl", "de het een en is ben zijn ik jij je wij we ze voor met in op aan van dat die wie iemand hallo bedankt dank alsjeblieft graag kan me mij nog ook niet zoek hebben nodig terug wacht even avond fijne groep allemaal zin over deze verkoop koop prijs goud fluister nu alleen ook heel waar wanneer ga goed"),
    ("pl", "i w na z do że to jest jestem są ja ty my wy ktoś cześć dzięki dziękuję proszę czy może mi mnie jeszcze też nie tak szukam potrzebujemy wracam poczekajcie grupę wszystkim za jednego minut tym sprzedam kupię złoto cena pw piszcie teraz tylko bardzo można trzeba gdzie kiedy idę będę dobrze"),
    ("cs", "a v na s do že to je jsem jsou já ty my vy někdo ahoj díky děkuji prosím může mi mě ještě taky ne ano hledám potřebujeme zpátky počkejte skupinu všichni jít večer za nechce tímhle pomoct hezký"),
    ("sk", "a v na s do že to je som sú ja ty my vy niekto ahoj ahojte vďaka ďakujem prosím môže mi ma mňa ešte tiež nie áno hľadám potrebujeme späť počkajte skupinu všetci ísť večer za nejde touto pomôcť pekný"),
    ("hu", "a az egy és is van vagyok vagy én te mi ti valaki szia sziasztok köszi köszönöm kérlek tud nekem még nem igen keresek kell vissza várjatok csoport csoportot este estét perc múlva jön jövök meg ebben segíteni"),
    ("ro", "și în la cu de pe că este sunt eu tu noi voi cineva salut mulțumesc mersi te rog poate mă încă nu da caut avem nevoie revin așteptați grup seara seară tuturor un o mai vine pentru bună faci bine foarte vreau acum unde"),
    ("bg", "и в на с за от да е съм са аз ти ние вие някой здравейте здрасти благодаря моля може ли ми още не търся трябва ни връщам изчакайте групата вечер след тази това всички продавам купувам злато цена пишете сега само също много къде кога добре ще какво нещо искам минути"),
    ("el", "και το τα η ο οι ένα μια σε για με από που είναι είμαι εγώ εσύ εμείς κάποιος κανείς γεια σας ευχαριστώ παρακαλώ μπορεί μου ακόμα δεν ναι ψάχνω χρειαζόμαστε ομάδα βράδυ"),
    ("hr", "i u na s sa za od da je sam su ja ti mi vi netko tko što bok hvala molim može li me još ne tražim trebamo vraćam pričekajte grupu grupi večer navečer svima koji ovim ugodna ugodan pomoći"),
    ("sr", "i u na s sa za od da je sam su ja ti mi vi neko ko šta zdravo hvala molim može li me još ne tražim treba vraćam sačekajte grupu veče svima koji и у на с са за од да је сам су ја ти ми ви неко ко шта здраво хвала молим може ли ме још не тражим треба враћам сачекајте групи вече свима који која"),
    ("sl", "in v na s z za od da je sem so jaz ti mi vi kdo živjo zdravo hvala prosim lahko me še ne iščem potrebujemo vrnem počakajte skupino skupini večer vsi ki gre čez"),
    ("uk", "і й в у на з за від що це є я ти ми ви хтось хто привіт дякую будь ласка може мені мене ще не так шукаю потрібен повернуся зачекайте групу вечора усім який яка нам продам куплю золото ціна пишіть зараз тільки теж дуже можна треба давай де коли хвилин піду буду поки добре"),
    ("ru", "и в на с за от что это есть я ты мы вы кто нибудь привет спасибо пожалуйста может мне меня еще ещё не да ищу нужен вернусь подождите группу вечера всем который которая нам через продам куплю золото цена пишите лс сначала уже сейчас только тоже очень можно нужно надо давай где когда минут пойду буду пока хорошо отойду стойте"),
    ("sv", "och i på med för av att det den en ett är jag du vi ni någon hej tack snälla kan mig mitt fortfarande också inte ja letar behöver tillbaka vänta gruppen kväll kvällarna allihop som till här"),
    ("no", "og i på med for av at det den en et er jeg du vi dere noen hei takk kan meg fortsatt også ikke ja ser etter trenger tilbake vent gruppen kveld kveldene alle sammen som til fin dette"),
    ("da", "og i på med for af at det den en et er jeg du vi nogen hej tak venligst kan mig stadig også ikke ja leder efter mangler tilbage vent gruppen aften aftenen alle sammen som der til god hav denne"),
    ("fi", "ja on olen ovat minä sinä me te joku moi hei kiitos ole hyvä voi voiko minulle minua vielä myös ei kyllä etsin tarvitsemme palaan odottakaa ryhmä ryhmästä illalla kaikki joka tässä"),
];

/// Mots courants du chat de jeu ajoutés en 0.13.3 (« great job » restait non traduit : aucun de
/// ses mots n'était connu). Même règle : un mot partagé compte pour chaque langue.
const MORE_WORDS: &[(&str, &str)] = &[
    ("en", "invite invites great job nice well done thats that's wow awesome amazing omg lmao haha hahaha gj wp gratz congrats grats thx tyvm ty sry soz idk idc ikr imo tbh btw anyway anyways guess gonna wanna gotta lemme kinda sorta yall y'all ya yo sup man men woman kill killed die died dead boss mob mobs pull pulled wipe wiped carry carried sell selling buy buying cheap free fast slow easy hard old young first last next best worst big small new long short high low left right still yet ever since until after before during while over under again almost already alone together team guild party member members player players server realm horde alliance level levels mount mounts achievement achievements spot spawn rare elite drop drops item items key keys run runs need needs want wants anybody somebody everyone someone whats what's where's who's how's it's he's she's we're they're you're i'll you'll we'll i've you've i'd didn't doesn't isn't aren't wasn't won't wouldn't couldn't shouldn't haven't hasn't ok then stuff thing things life time times day days week year hour hours minute game games play playing played"),
    ("de", "danke schön super toll geil krass gut gemacht gerne leider genau stimmt jetzt heute morgen gestern schon noch mehr weniger viel viele wenig nichts alles jeder jemand niemand mein dein sein unser euer hab habe haben hast kein keine nix bitteschön tschüss moin servus na ne nee jo jup ok lass lasst komm kommt geht gehen gehe machen mach macht spielen spiel spieler gilde boss kaputt tot gestorben schnell langsam einfach schwer neu alt groß klein erste letzte nächste beste"),
    ("es", "gracias genial bien hecho bueno buena vale venga vamos claro jaja jajaja jeje tío tia hermano amigo amigos ahora hoy mañana ayer ya aún más menos mucho muchos poco nada todo alguien nadie mi tu su nuestro tengo tienes tiene hay hacer hago juego jugar jugador hermandad jefe muerto rápido lento fácil difícil nuevo viejo grande pequeño primero último siguiente mejor peor"),
    ("it", "grazie grande bravo bravi bene fatto bello bella dai andiamo certo ahah ahahah ragazzi amico amici adesso oggi domani ieri già ancora più meno molto molti poco niente tutto qualcuno nessuno mio tuo suo nostro ho hai ha c'è fare faccio gioco giocare giocatore gilda boss morto veloce lento facile difficile nuovo vecchio grande piccolo primo ultimo prossimo migliore peggiore"),
    ("pt", "obrigado valeu legal ótimo bom boa feito vamos claro kkk kkkk haha mano amigo amigos agora hoje amanhã ontem já ainda mais menos muito muitos pouco nada tudo alguém ninguém meu teu seu nosso tenho tens tem fazer faço jogo jogar jogador guilda chefe morto rápido lento fácil difícil novo velho grande pequeno primeiro último próximo melhor pior"),
    ("nl", "bedankt super mooi goed gedaan leuk kom komen gaan doe doen nu vandaag morgen gisteren al nog meer minder veel weinig niets alles iedereen niemand mijn jouw zijn ons heb heeft hebt geen spel spelen speler gilde baas dood snel langzaam makkelijk moeilijk nieuw oud groot klein eerste laatste volgende beste"),
    ("pl", "dzięki dzieki super dobra dobrze świetnie spoko siema elo nara jutro dzisiaj wczoraj już jeszcze więcej mniej dużo mało nic wszystko ktoś nikt mój twój jego nasz mam masz ma gra grać gracz gildia boss martwy szybko wolno łatwy trudny nowy stary duży mały pierwszy ostatni następny najlepszy"),
    ("ru", "спс спасибки норм нормально круто класс отлично молодцы молодец го давайте ребята народ сегодня завтра вчера уже ещё больше меньше много мало ничего всё все кто-то никто мой твой его наш есть нет игра играть игрок гильдия босс умер быстро медленно легко сложно новый старый большой маленький первый последний следующий лучший"),
];

/// Lettres qui désignent une langue (ou une poignée), avec leur poids.
const LETTERS: &[(char, &[&str], f32)] = &[
    ('ș', &["ro"], 3.0), ('ş', &["ro"], 3.0), ('ț', &["ro"], 3.0), ('ţ', &["ro"], 3.0), ('ă', &["ro"], 3.0),
    ('ő', &["hu"], 3.0), ('ű', &["hu"], 3.0),
    ('ř', &["cs"], 3.0), ('ů', &["cs"], 3.0), ('ě', &["cs"], 3.0),
    ('ľ', &["sk"], 3.0), ('ĺ', &["sk"], 3.0), ('ŕ', &["sk"], 3.0), ('ô', &["sk"], 1.0), ('ä', &["sk", "de", "sv", "fi"], 0.5),
    ('ł', &["pl"], 3.0), ('ą', &["pl"], 3.0), ('ę', &["pl"], 3.0), ('ś', &["pl"], 3.0), ('ź', &["pl"], 3.0), ('ż', &["pl"], 3.0),
    ('ń', &["pl"], 2.0),
    ('đ', &["hr", "sr"], 3.0), ('ć', &["hr", "sr", "pl"], 1.0),
    ('č', &["hr", "sr", "sl", "cs", "sk"], 0.5), ('š', &["hr", "sr", "sl", "cs", "sk"], 0.5), ('ž', &["hr", "sr", "sl", "cs", "sk"], 0.5),
    ('ñ', &["es"], 3.0), ('¿', &["es"], 3.0), ('¡', &["es"], 3.0),
    ('ã', &["pt"], 3.0), ('õ', &["pt"], 3.0), ('ç', &["pt", "fr"], 1.0),
    ('œ', &["fr"], 3.0), ('ê', &["fr", "pt"], 0.5), ('è', &["fr", "it"], 0.5), ('à', &["fr", "it", "pt"], 0.3),
    ('ù', &["fr", "it"], 0.5), ('î', &["fr", "ro"], 0.5), ('â', &["fr", "ro", "pt"], 0.3), ('û', &["fr"], 1.0),
    ('ì', &["it"], 1.0), ('ò', &["it"], 1.0),
    ('ß', &["de"], 3.0), ('ü', &["de", "hu"], 0.5), ('ö', &["de", "sv", "fi", "hu"], 0.5),
    ('å', &["sv", "no", "da"], 1.0), ('æ', &["no", "da"], 2.0), ('ø', &["no", "da"], 2.0),
    // Cyrillique.
    ('ы', &["ru"], 3.0), ('э', &["ru"], 3.0), ('ё', &["ru"], 3.0),
    ('і', &["uk"], 3.0), ('ї', &["uk"], 3.0), ('є', &["uk"], 3.0), ('ґ', &["uk"], 3.0),
    ('ђ', &["sr"], 3.0), ('ћ', &["sr"], 3.0), ('љ', &["sr"], 3.0), ('њ', &["sr"], 3.0), ('џ', &["sr"], 3.0), ('ј', &["sr"], 3.0),
    ('ъ', &["bg"], 2.0), ('ь', &["ru", "uk"], 0.7),
];

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

fn is_greek(c: char) -> bool {
    ('\u{0370}'..='\u{03FF}').contains(&c) || ('\u{1F00}'..='\u{1FFF}').contains(&c)
}

/// Retire le balisage du chat (couleurs, liens, icônes) et les liens d'objets entre crochets.
pub fn strip_markup(text: &str) -> String {
    let items = crate::mirror::parse_markup(text, [255, 255, 255]);
    let mut s: String = items
        .iter()
        .map(|it| match it {
            crate::mirror::Item::Char(c, _) => *c,
            _ => ' ',
        })
        .collect();
    // Noms d'objets, de sorts, de quêtes entre crochets : dans la langue du client de l'autre.
    while let (Some(a), Some(b)) = (s.find('['), s.find(']')) {
        if b <= a {
            break;
        }
        s.replace_range(a..=b, " ");
    }
    s
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

fn whatlang_code(l: whatlang::Lang) -> Option<&'static str> {
    use whatlang::Lang::*;
    Some(match l {
        Fra => "fr", Eng => "en", Deu => "de", Spa => "es", Ita => "it", Rus => "ru", Por => "pt", Nld => "nl", Pol => "pl",
        Ces => "cs", Slk => "sk", Hun => "hu", Ron => "ro", Bul => "bg", Ell => "el", Hrv => "hr", Srp => "sr", Slv => "sl",
        Ukr => "uk", Swe => "sv", Nob => "no", Fin => "fi", Dan => "da",
        _ => return None,
    })
}

fn detector() -> &'static whatlang::Detector {
    use std::sync::OnceLock;
    static D: OnceLock<whatlang::Detector> = OnceLock::new();
    D.get_or_init(|| {
        use whatlang::Lang::*;
        whatlang::Detector::with_allowlist(vec![
            Fra, Eng, Deu, Spa, Ita, Rus, Por, Nld, Pol, Ces, Slk, Hun, Ron, Bul, Ell, Hrv, Srp, Slv, Ukr, Swe, Nob, Fin, Dan,
        ])
    })
}

fn word_table() -> &'static HashMap<&'static str, Vec<&'static str>> {
    use std::sync::OnceLock;
    static T: OnceLock<HashMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        for (lang, list) in WORDS.iter().chain(MORE_WORDS) {
            for w in list.split_whitespace() {
                let v = t.entry(w).or_default();
                if !v.contains(lang) {
                    v.push(*lang);
                }
            }
        }
        t
    })
}

/// Distance d'édition au plus 1 (lettre en trop, en moins, remplacée, ou deux lettres voisines
/// échangées) entre deux mots.
fn one_edit(a: &[char], b: &[char]) -> bool {
    let (la, lb) = (a.len(), b.len());
    if la.abs_diff(lb) > 1 {
        return false;
    }
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    if p == la && p == lb {
        return true;
    }
    if la == lb {
        // Remplacement, ou échange de deux lettres voisines.
        return a[p + 1..] == b[p + 1..] || (p + 1 < la && a[p] == b[p + 1] && a[p + 1] == b[p] && a[p + 2..] == b[p + 2..]);
    }
    let (long, short) = if la > lb { (a, b) } else { (b, a) };
    long[p + 1..] == short[p..]
}

/// Mot connu le plus proche d'un mot mal écrit (4 lettres au moins, pour ne pas confondre les
/// mots courts entre langues) : ses langues, si un seul mot connu est à une faute près.
fn near_word(w: &str) -> Option<&'static Vec<&'static str>> {
    let wc: Vec<char> = w.chars().collect();
    if wc.len() < 4 {
        return None;
    }
    use std::sync::OnceLock;
    // Mots connus de 4 lettres et plus, découpés en lettres une fois pour toutes.
    static LONG: OnceLock<Vec<(Vec<char>, &'static Vec<&'static str>)>> = OnceLock::new();
    let long = LONG.get_or_init(|| {
        word_table().iter().map(|(k, l)| (k.chars().collect::<Vec<char>>(), l)).filter(|(k, _)| k.len() >= 4).collect()
    });
    let mut found: Option<&'static Vec<&'static str>> = None;
    for (kc, langs) in long {
        if kc[0] != wc[0] && kc.get(1) != wc.get(1) {
            continue;
        }
        if one_edit(&wc, kc) {
            match found {
                // Deux mots proches de langues différentes : trop incertain.
                Some(prev) if prev != *langs => return None,
                _ => found = Some(langs),
            }
        }
    }
    found
}

/// Points de chaque langue pour un message (pour les tests et le réglage).
pub fn scores(text: &str) -> Vec<(&'static str, f32)> {
    scored(text, true)
}

/// Points des seuls indices propres au chat (mots et lettres), sans le vote du détecteur général.
fn evidence(text: &str) -> Vec<(&'static str, f32)> {
    scored(text, false)
}

fn scored(text: &str, vote: bool) -> Vec<(&'static str, f32)> {
    let clean = strip_markup(text);
    let ws: Vec<String> = words(&clean).into_iter().filter(|w| !JARGON.contains(&w.as_str())).collect();
    let mut sc: HashMap<&'static str, f32> = HashMap::new();
    let table = word_table();
    for w in &ws {
        if let Some(langs) = table.get(w.as_str()) {
            // Un mot partagé par beaucoup de langues dit peu ; un mot d'une lettre encore moins.
            let weight = if w.chars().count() == 1 { 0.3 } else { 1.0 } / (langs.len() as f32).sqrt();
            for l in langs {
                *sc.entry(l).or_default() += weight;
            }
        } else if let Some(langs) = near_word(w) {
            // Faute de frappe ou d'orthographe (0.13.3) : « thnaks », « pleas », « somone » comptent,
            // un peu moins qu'un mot juste.
            let weight = 0.6 / (langs.len() as f32).sqrt();
            for l in langs {
                *sc.entry(l).or_default() += weight;
            }
        }
    }
    let lower = clean.to_lowercase();
    let mut seen = Vec::new();
    for c in lower.chars() {
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        if let Some((_, langs, w)) = LETTERS.iter().find(|(l, _, _)| *l == c) {
            for l in langs.iter() {
                *sc.entry(*l).or_default() += *w;
            }
        }
    }
    // Écriture : cyrillique et grec écartent les autres langues.
    let cyr = lower.chars().filter(|c| is_cyrillic(*c)).count();
    let grk = lower.chars().filter(|c| is_greek(*c)).count();
    let latin = lower.chars().filter(|c| c.is_alphabetic() && !is_cyrillic(*c) && !is_greek(*c)).count();
    if grk > latin.max(cyr) {
        sc.retain(|l, _| *l == "el");
        *sc.entry("el").or_default() += 5.0;
    } else if cyr > latin {
        sc.retain(|l, _| ["ru", "uk", "bg", "sr"].contains(l));
        for l in ["ru", "uk", "bg", "sr"] {
            *sc.entry(l).or_default() += 0.5;
        }
    } else {
        sc.retain(|l, _| !["ru", "uk", "bg", "el"].contains(l) || latin == 0);
    }
    // Le détecteur général vote aussi.
    if let Some(info) = detector().detect(&ws.join(" ")).filter(|_| vote) {
        if let Some(code) = whatlang_code(info.lang()) {
            if sc.contains_key(code) || sc.is_empty() {
                // Vote modéré : sur un message court, il départage sans écraser les indices.
                *sc.entry(code).or_default() += 0.5 + 0.5 * info.confidence() as f32;
            }
        }
    }
    let mut v: Vec<(&'static str, f32)> = sc.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    v
}

/// Langue d'un message.
pub fn detect(text: &str) -> Detected {
    let clean = strip_markup(text);
    let letters: usize = words(&clean)
        .iter()
        .filter(|w| !JARGON.contains(&w.as_str()))
        .map(|w| w.chars().count())
        .sum();
    // Trop peu de lettres hors jargon : rien à traduire.
    if letters < 4 {
        return Detected::Neutral;
    }
    let sc = scores(text);
    let Some(&(best, top)) = sc.first() else { return Detected::Neutral };
    let second = sc.get(1).map_or(0.0, |s| s.1);
    // Un mot propre à une seule langue suffit (« hello », « danke ») ; un mot partagé par
    // plusieurs langues, ou inconnu, ne suffit pas.
    if top < 1.0 {
        return Detected::Neutral;
    }
    let conf = ((top - second) / top).clamp(0.0, 1.0) * (top / 4.0).min(1.0);
    Detected::Lang(best, conf)
}

/// Langue d'un message lu par un joueur dont le client est en `client` : un message incertain qui
/// peut être dans la langue du client est dans la langue du client (« salut » : français pour un
/// client français, pas roumain), pour ne jamais traduire ce que le joueur lit déjà.
pub fn detect_for(text: &str, client: &str) -> Detected {
    let d = detect(text);
    match d {
        Detected::Lang(l, c) if l != client && c < UNSURE && client_plausible(text, client) => Detected::Lang(lang_code(client), c),
        _ => d,
    }
}

/// En dessous de cette confiance, un message est incertain.
const UNSURE: f32 = 0.35;

/// Le message peut-il être dans la langue du client ? Oui si ses mots et ses lettres le disent
/// presque autant que ceux de la langue en tête (« salut » : autant français que roumain ;
/// « Salut tuturor, ce mai faci? » : nettement roumain).
fn client_plausible(text: &str, client: &str) -> bool {
    let ev = evidence(text);
    let top = ev.first().map_or(0.0, |e| e.1);
    ev.iter().any(|(x, s)| *x == client && *s >= 0.5 && *s >= 0.8 * top)
}

fn lang_code(code: &str) -> &'static str {
    LANGS.iter().find(|l| **l == code).copied().unwrap_or("en")
}

/// Langue de chaque joueur, apprise sur ses messages : départage les messages incertains, et
/// donne la langue cible d'une réponse (Trad:AUTO).
#[derive(Default)]
pub struct Speakers {
    by_name: HashMap<String, VecDeque<&'static str>>,
}

impl Speakers {
    /// Langue retenue pour un message de `who` lu par un client en `client`, et mémorisée.
    pub fn observe(&mut self, who: &str, text: &str, client: &str) -> Detected {
        let d = detect(text);
        let known = self.language_of(who);
        let out = match (&d, known) {
            // Message incertain qui peut être dans la langue du client : il l'est (rien à traduire).
            (Detected::Lang(l, c), _) if *c < UNSURE && *l != client && client_plausible(text, client) => {
                Detected::Lang(lang_code(client), *c)
            }
            // Sinon, la langue habituelle du joueur, si elle est parmi les candidates, l'emporte.
            (Detected::Lang(l, c), Some(k)) if *c < UNSURE && *l != k => {
                let sc = scores(text);
                let top = sc.first().map_or(0.0, |s| s.1);
                match sc.iter().find(|(x, _)| *x == k) {
                    Some((_, s)) if *s >= top * 0.6 => Detected::Lang(k, 0.35),
                    _ => d.clone(),
                }
            }
            _ => d.clone(),
        };
        if let Detected::Lang(l, c) = out {
            if c >= 0.2 {
                let q = self.by_name.entry(who.to_string()).or_default();
                q.push_back(l);
                if q.len() > 8 {
                    q.pop_front();
                }
            }
        }
        out
    }

    /// Langue la plus fréquente des derniers messages de `who`.
    pub fn language_of(&self, who: &str) -> Option<&'static str> {
        let q = self.by_name.get(who)?;
        let mut best: Option<(&'static str, usize)> = None;
        for l in q {
            let n = q.iter().filter(|x| *x == l).count();
            if best.is_none_or(|(_, b)| n > b) {
                best = Some((l, n));
            }
        }
        best.map(|(l, _)| l)
    }
}

/// Code de langue à partir de la langue du client de WoW (frFR, enGB, deDE…).
pub fn from_locale(locale: &str) -> Option<&'static str> {
    let code = locale.get(..2)?.to_lowercase();
    LANGS.iter().find(|l| **l == code).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: &str = include_str!("../bench/langues.tsv");

    #[test]
    fn all_23_languages_on_short_chat_messages() {
        let mut wrong = Vec::new();
        let mut n = 0;
        for l in SAMPLES.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let (want, msg) = l.split_once('\t').unwrap();
            n += 1;
            let got = detect(msg);
            if got.code() != Some(want) {
                wrong.push(format!("{want} -> {got:?} : {msg} {:?}", &scores(msg)[..scores(msg).len().min(3)]));
            }
        }
        assert_eq!(n, 138);
        // whatlang seul : 123 sur 138 (09/11/2026).
        assert!(wrong.is_empty(), "{} erreurs sur {n} :\n{}", wrong.len(), wrong.join("\n"));
    }

    #[test]
    fn game_chat_words_and_typos() {
        // Messages restés non traduits en 0.13.2 (capture du 10/11/2026).
        for t in ["great job borger !", "nice one guys", "well done everyone"] {
            assert!(matches!(detect(t), Detected::Lang("en", _)), "{t} : {:?}", scores(t));
        }
        // Fautes de frappe : une lettre en trop, en moins, remplacée ou deux lettres échangées.
        for t in ["thnaks for the hepl", "somone selling flasks", "pleas invite me"] {
            assert!(matches!(detect(t), Detected::Lang("en", _)), "{t} : {:?}", scores(t));
        }
        assert!(matches!(detect("dnake schön"), Detected::Lang("de", _)), "{:?}", scores("dnake schön"));
        let c = |s: &str| s.chars().collect::<Vec<char>>();
        assert!(one_edit(&c("thnaks"), &c("thanks")));
        assert!(one_edit(&c("pleas"), &c("please")));
        assert!(one_edit(&c("helo"), &c("hello")));
        assert!(!one_edit(&c("thx"), &c("thanks")));
        // Mots courts : jamais rapprochés (« tak » n'est pas « tank »).
        assert!(near_word("tak").is_none());
    }

    #[test]
    fn jargon_and_links_are_neutral() {
        for m in ["LF2M tank heal MC", "WTS |cffa335ee|Hitem:19019::::::::60:::::|h[Thunderfury, Blessed Blade of the Windseeker]|h|r pst", "gg", "inv pls", "lol", "123", "+1"] {
            assert_eq!(detect(m), Detected::Neutral, "{m}");
        }
        // Messages anglais courts du chat général (essai en jeu du 09/11/2026).
        for m in ["but i personally like it", "Much better then Dorutar", "ouais parce que personne ne l'entend"] {
            assert!(detect(m).code().is_some(), "{m}");
        }
        assert_eq!(detect("but i personally like it").code(), Some("en"));
        assert_eq!(detect("Much better then Dorutar").code(), Some("en"));
        // Le jargon ne fausse pas la langue du reste.
        assert_eq!(detect("LF tank et heal pour Cœur du Magma, on part dans 10 min").code(), Some("fr"));
        assert_eq!(detect("LFM Molten Core, need 1 tank and 2 healers, PST").code(), Some("en"));
        assert_eq!(detect("Suche Gruppe für die Quest im Schlingendorntal, bin Stufe 38.").code(), Some("de"));
    }

    #[test]
    fn a_speaker_settles_uncertain_messages() {
        let mut s = Speakers::default();
        for m in ["takk for gruppen, ha en fin kveld", "kan noen hjelpe meg med dette oppdraget?"] {
            assert_eq!(s.observe("Olav", m, "fr").code(), Some("no"));
        }
        assert_eq!(s.language_of("Olav"), Some("no"));
        // Message court et ambigu : la langue habituelle de Olav l'emporte.
        assert_eq!(s.observe("Olav", "vi trenger en til", "fr").code(), Some("no"));
        // Essai en jeu du 09/11/2026 : un message roumain, puis « salut » (français pour un client
        // français, même si le joueur vient d'écrire en roumain).
        assert_eq!(s.observe("Hazè", "Salut tuturor, ce mai faci?", "fr").code(), Some("ro"));
        assert_eq!(s.observe("Hazè", "salut", "fr").code(), Some("fr"));
        assert_eq!(detect_for("salut", "fr").code(), Some("fr"));
        assert_eq!(detect_for("hello boy", "fr").code(), Some("en"));
        // Essai en jeu du 09/11/2026 : « hello » seul, écrit par le joueur, est de l'anglais.
        assert_eq!(detect_for("hello", "fr").code(), Some("en"));
        assert_eq!(detect_for("danke", "fr").code(), Some("de"));
        assert_eq!(detect_for("comment allez vous?", "fr").code(), Some("fr"));
        assert_eq!(from_locale("frFR"), Some("fr"));
        assert_eq!(from_locale("enGB"), Some("en"));
        assert_eq!(from_locale("zhCN"), None);
    }
}
#[cfg(test)]
mod corpus {
    /// Les messages du banc (7 langues, argot du jeu, liens d'objets) : tous reconnus.
    #[test]
    fn bench_corpus_messages() {
        let c = include_str!("../bench/corpus.tsv");
        let mut wrong = Vec::new();
        for l in c.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let f: Vec<&str> = l.splitn(3, '\t').collect();
            let d = super::detect(f[2]);
            if d.code() != Some(f[0]) {
                wrong.push(format!("{} -> {:?} : {}", f[0], d, f[2]));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}
