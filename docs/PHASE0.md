> **Archive de la phase 0** (kit de test v0.12.6, recopié tel quel). Depuis la 0.13 : l'outil
> `ct-spike` s'appelle `ct-outils` (dossier `app`, `cargo run --release --bin ct-outils -- <commande>`),
> l'addon `CT_Spike` s'appelle `ChevaliersTranslators`, `/ctspike` devient `/chevaliers`, et les
> emplacements S6 (`slot`) ont été retirés. Le produit est décrit dans le `README.md` du dépôt.

# Chevaliers Translators - Phase 0 (v0.12.6)

Kit de test des points éliminatoires avant d'écrire le produit :

| Test | Ce qu'il prouve | Outil |
| --- | --- | --- |
| S1 | Le bandeau de pixels passe de WoW à Windows sans erreur, assez vite | `/ctspike test` + `ct-spike live` |
| S2 | Le chat reçu sort du jeu (et les messages « secrets » sont ignorés) | `/ctspike chat on` + `ct-spike live` |
| S6 | WoW relit un emplacement réécrit pendant la partie, et peut l'envoyer | `ct-spike slot write` + `/ctspike slot` |
| S3 | Maj+Entrée : la traduction est collée dans la boîte de saisie et envoyée, sur tous les canaux | `ct-spike live --wow` + chat |
| S7 | Coexistence avec QuestVoice (bandeau QuestVoice en haut à gauche, CT en bas à droite) | les deux actifs |
| S5a | L'application retrouve la police exacte du chat dans les fichiers du jeu, et sait quelles langues elle dessine | `ct-spike fonts` + `/ctspike font` |
| S5 | Chat miroir : le texte officiel est caché, l'application le redessine au même endroit, même police, même effacement | `/ctspike miroir` + `ct-spike live` |
| S4a | Le matériel de chaque joueur est détecté pour choisir où traduire (carte graphique ou processeur) et quelle taille de modèle | `ct-spike materiel` |
| S4b | Comparatif des modèles de traduction et test en jeu : vitesse, mémoire, qualité, images par seconde, à-coups, stabilité du pilote graphique | `ct-spike bench` + llama.cpp |
| S4c | Modèles chez le joueur : pack low d'office et en repli, autres modèles sur demande classés par mémoire vidéo, un seul actif, interface graphique | `ct-spike interface` |
| S8 | Traduction réelle de bout en bout : langue de chaque message détectée, messages étrangers traduits dans le chat miroir (« [DE] … »), Maj+Entrée traduit vers Trad:AUTO ou Trad:XX, chaîne de repli en usage réel | `ct-spike live` (active d'office à la connexion) |
| S9 | Retail : canaux du chat dans le miroir, et réglages de traduction propres à retail (tests de performance au maximum des réglages) | `/ctspike diag`, `ct-spike tests-en-jeu` + `/ctspike essai` dans retail |

Pas encore dans ce kit : capture par Windows Graphics Capture (S1b), repli sur une baisse
d'images par seconde mesurée en jeu pendant l'usage normal (les plantages, le manque de mémoire et
la lenteur sont déjà suivis), dictionnaire des termes du jeu au-delà des donjons et raids de base.

## Contenu

```
ct-phase0/
  addon/CT_Spike/       addon de test (Lua 5.1, toutes versions)
  ct-spike/             outil Rust : auto-test, décodeur en direct, installation, emplacements, polices
  tools/lua_harness.py  harnais hors jeu (imitation de l'API WoW), utilisé pour la vérification
```

## Vérifié avant livraison

- L'encodeur Lua (Lua 5.1 et LuaJIT) et l'encodeur Rust produisent des trames identiques (62 vecteurs).
- Auto-test : 3000 trames en conditions réalistes (gamma, contraste, luminosité, bruit, flou,
  décalage) toutes décodées ; aucune trame altérée acceptée avec un contenu faux.
- L'addon exécuté dans une imitation de l'API WoW : ses trames peintes sont décodées par l'outil
  (test, ping, chat), un message secret n'est pas transmis, un emplacement est lu puis envoyé.
- Le code de capture Windows est vérifié contre les signatures de `windows-sys`,
  mais n'a pas pu être exécuté sous Windows : c'est l'objet du test S1.
- Lecteur des archives du jeu (S5a) : une fausse installation au format du jeu, écrite par l'outil,
  est relue entièrement (403/403 fichiers, MD5 justes) par `casc-lib`, une bibliothèque indépendante
  éprouvée sur de vraies installations ; l'outil la relit aussi, y compris la variante française
  d'un fichier, le repli sur une seconde copie et le refus d'un fichier altéré.
- Analyse des polices : nombre de caractères et lettres manquantes identiques à `fontTools`
  sur trois polices réelles (DejaVu Sans, Liberation Sans, WenQuanYi).
- Mise en page du miroir réglée sur les mesures faites dans le jeu (0.4.1, 0.4.2) : l'addon mesure
  avec la police du chat, à sa taille réelle, l'avance de chaque caractère affiché, le crénage de
  chaque paire de lettres affichée et le retrait des lignes coupées (lu sur une vraie ligne coupée) ;
  l'outil s'en sert pour couper et placer le texte.
- Chat miroir, addon exécuté dans une imitation de la fenêtre de chat de Blizzard : texte officiel
  caché puis rétabli (ligne secrète, verrouillage, arrêt), journal de combat jamais touché ; ses
  descriptions passent par le bandeau et l'outil en tire une image fidèle (couleurs, liens,
  icônes, coupure des lignes, latin, cyrillique, grec, polonais, roumain).
- Fenêtres du miroir (Windows) vérifiées contre les signatures de `windows-sys`, pas encore exécutées.
- 0.11 : tout le code Windows (collage et Entrée simulés, presse-papiers, fenêtres des icônes de
  copie, version du jeu lue sur son exécutable) compilé contre `windows-sys` avec la configuration
  Windows forcée ; le parcours Maj+Entrée (demande, boîte rouverte sur le même canal et le même
  destinataire, collage prévu, envoi, annulation par la frappe, texte remis au bout de 6 s), le
  menu à un niveau, la bulle d'aide signalée, le diagnostic et le démarrage automatique sont joués
  par l'addon dans le harnais, trames décodées par l'outil.
- Fenêtre de l'interface (0.8.1) vérifiée contre les vraies bibliothèques `wry` et `tao` ;
  génération par rafales testée avec un faux serveur llama.cpp (reprise exacte du texte, gabarit
  de discussion appliqué par le serveur, repli d'une traite si le serveur ne sait pas le faire).
- Interface des modèles : serveur local et page testés en démonstration (état, téléchargement
  simulé, activation, suppression, refus sans jeton), pages capturées en thème sombre et clair et en
  fenêtre étroite. Lecture des journaux de Windows testée sur les vrais messages du 09/11/2026.
  Priorité de la carte et relevés de mémoire vidéo vérifiés contre les signatures de `windows-sys`.

## Commandes de test (PowerShell)

Tout se fait dans PowerShell : l'outil compile et tourne nativement sous Windows.
Adapter les deux chemins de la première ligne, puis garder la même fenêtre.

```powershell
$wow = "D:\World of Warcraft"; $ct = "C:\Dev\ChevaliersTranslators"
```

### 1. Poser les fichiers

```powershell
New-Item -ItemType Directory -Force $ct | Out-Null
Expand-Archive -Force "$env:USERPROFILE\Downloads\ct-phase0-v0.12.6.zip" $ct
cd "$ct\ct-phase0\ct-spike"
```

### 2. Auto-test sans le jeu

```powershell
cargo test --release --target-dir target-win
cargo run --release --target-dir target-win -- selftest
```

Attendu : `84 passed` (85 sous Linux) puis `RÉSULTAT : OK`. La première compilation de la 0.8.1 est plus longue
(fenêtre de l'interface : bibliothèques `wry` et `tao`).

### 3. Installer l'addon et les emplacements (WoW fermé)

**0.11 : quitter complètement WoW avant l'installation** (nouvel addon `CT_Spike_Live` : la liste
des addons n'est lue qu'au lancement du jeu). Sans `--wow`, l'installation
trouve elle-même WoW (registre de Windows, emplacements courants) ; sans `--flavor`, elle installe
dans **toutes les versions du jeu présentes**. `ct-spike addon` affiche l'état de l'addon dans
chaque version (absent, à jour, ancienne version).

Quitter complètement WoW avant cette étape quand l'addon reçoit un nouveau fichier
(c'est le cas depuis 0.4.0 : `Mirror.lua`) : la liste des fichiers n'est lue qu'au lancement du jeu.
Sinon, réinstaller pendant que WoW tourne puis `/reload` suffit.

```powershell
cargo run --release --target-dir target-win -- install --wow $wow --flavor _forever_
```

- Le numéro d'interface est lu dans `.build.info` (produit `wowf` pour Forever). Si la version
  n'est pas reconnue, ajouter `--interface 16001` (Forever), `120105` (Retail), `11509` (Classic Era).
- Sans `--flavor`, l'outil installe dans toutes les versions trouvées.
- Une réinstallation pendant que WoW tourne met à jour les fichiers Lua : `/reload` suffit.

### 4. Décodeur en direct (seconde fenêtre PowerShell)

```powershell
$wow = "D:\World of Warcraft"
cd "C:\Dev\ChevaliersTranslators\ct-phase0\ct-spike"
cargo run --release --target-dir target-win -- live --wow $wow --flavor _forever_
```

`--wow` et `--flavor` servent à S3 (écriture des traductions dans les emplacements) et à S5
(police du chat lue dans les fichiers du jeu) ; sans eux, le décodeur fonctionne pour S1 et S2,
copie seulement les traductions dans le presse-papiers, et le miroir utilise Arial.

`--target-dir target-win` évite de recompiler entre deux commandes.
WoW doit être en mode **Fenêtré (plein écran)** sur l'écran principal.

Depuis la 0.9, le décodeur **traduit pour de vrai** (S8) : ajouter `--dossier` (dossier des
modèles) ; `--sans-traduction` revient à la traduction simulée `[→XX]`.

## Procédures

### S1 - canal pixel

1. Lancer le décodeur (étape 4), puis WoW, se connecter.
2. En jeu : `/ctspike info` (vérifie écran, colonnes, charge utile).
3. En jeu : `/ctspike test 1000 20` (50 s).
4. Le décodeur affiche le résultat 3 s après la dernière trame.

Critères Go : 0 contenu faux accepté, 0 trame perdue, capacité à pleine trame ≥ 2000 octets/s.
Les trames de test ont une longueur aléatoire : le débit utile mesuré vaut environ la moitié de la capacité.
Variantes : `/ctspike test 2000 30`, `/ctspike block 6`, échelle d'interface minimale et maximale.
Chaque trame reste au moins 2 images du jeu à l'écran : sous 40 FPS, la cadence réelle baisse.

### S2 - capture du chat

1. Décodeur lancé. En jeu : `/ctspike chat on`.
2. Écrire en guilde, groupe, chuchotement : chaque message apparaît dans le décodeur.
3. Retail ou Forever, verrouillage forcé : `/console addonChatRestrictionsForced 1`,
   écrire en guilde : rien ne doit sortir, et `/ctspike info` compte les « secrets ignorés ».
   Puis `/console addonChatRestrictionsForced 0`.

### S6 - emplacements chargés à la demande

Un emplacement ne se charge qu'une fois par session : il doit être réécrit **avant** son chargement,
et un `/reload` est nécessaire pour réutiliser les emplacements déjà chargés.

1. WoW lancé, personnage connecté. En jeu : `/dire test` (l'addon apprend ton nom complet ;
   sur Forever, `UnitName` ne renvoie que le prénom), puis `/ctspike info` (ligne « Noms »).
2. Remplir les 10 emplacements d'un coup :

```powershell
cargo run --release --target-dir target-win -- slot fill --wow $wow --flavor _forever_ --text "Hallo zusammen"
```

3. En jeu, enchaîner sans revenir à PowerShell, en comparant chaque jeton au tableau imprimé :
   `/ctspike slot` (lecture seule), puis `/ctspike slot whisper` (à soi-même), `/ctspike slot say`,
   `/ctspike slot guild`, `/ctspike slot party` (en groupe), `/ctspike slot instance` (en donjon).
   Pour chuchoter à quelqu'un d'autre : `/ctspike slot whisper Prénom Nom`.
4. Chaque envoi indique la langue utilisée : elle doit être celle de ta faction (Commun ou Orc),
   pas ta langue raciale.
5. Pour une nouvelle série : `/reload`, puis de nouveau `slot fill`.

Critères Go : bon jeton à chaque chargement, envoi accepté sur chaque canal, langue de faction.

### S3 - Maj+Entrée : traduction collée puis envoyée (0.11)

Décodeur lancé avec `--wow` et `--flavor` (étape 4), WoW au premier plan.

1. **Entrée intacte** : `/dire essai normal` + Entrée. Le message part normalement.
2. **Un appui** : `/w Elzhem Batonnoir` (toi-même), écrire `Bonjour à tous`, puis **Maj+Entrée**.
   La boîte se vide et se rouvre sur le même chuchotement ; la traduction y apparaît et part
   aussitôt. Tu reçois le message en allemand (Trad:DE) ; ta ligne « À [Elzhem] » s'affiche
   « [DE] Bonjour à tous ».
3. **Dire et canaux numérotés** : même chose sur `/dire` et `/2` : la traduction part en un appui
   (c'est ton Entrée, simulée : le jeu l'accepte partout).
4. **Lien d'objet** : Maj+clic sur un objet dans le message, puis Maj+Entrée : le lien part intact
   dans la traduction (collé sous forme de repère `{1}`, remis par l'addon juste avant l'envoi).
5. **Annulation** : Maj+Entrée puis taper tout de suite une lettre : rien ne part, la lettre reste
   dans la boîte (0.12.2 : sans message ; le collage, qui fait apparaître toute la traduction d'un
   coup, n'est plus pris pour une frappe).
6. **Application arrêtée** : Ctrl+C dans la console, puis Maj+Entrée : au bout de 6 s le texte
   revient dans la boîte (« Traduction pas arrivée à temps »).
7. `/ctspike info`, ligne « S3 » : demandes, envois, annulés, trop lents, brouillons.

Critères Go : Entrée jamais modifiée, aucun texte non traduit envoyé par Maj+Entrée, traduction
envoyée sur le bon canal au bon destinataire, un seul appui partout, presse-papiers du joueur
rendu après l'envoi.

### S5a - polices du chat (lecture des fichiers du jeu)

Le chat miroir doit dessiner exactement la police de WoW. L'outil la lit dans les archives
du jeu, **en lecture seule**, et la copie dans un cache sur ton PC
(`%LOCALAPPDATA%\ChevaliersTranslators\fonts`). Rien n'est écrit dans le dossier du jeu,
et les polices ne quittent jamais ton PC. WoW peut rester ouvert.

1. En jeu : `/ctspike font`. Note la police de chaque fenêtre de chat (attendu sans addon de chat :
   `Fonts\ARIALN.TTF 14 ombre 1,-1`), sa taille et sa position en pixels.
2. Dans PowerShell :

```powershell
cargo run --release --target-dir target-win -- fonts --wow $wow --flavor _forever_
```

3. Si `/ctspike font` affiche une autre police (addon de chat, ElvUI...), l'extraire aussi :

```powershell
cargo run --release --target-dir target-win -- fonts --wow $wow --flavor _forever_ --file "Fonts\ARIALN.TTF" --file "<chemin affiché en jeu>"
```

Attendu : pour chaque police, `MD5 vérifié`, le nom de la police (« Arial Narrow » pour ARIALN.TTF),
puis la couverture : les langues dont toutes les lettres existent (`complet : ...`) et, pour les
autres, les lettres manquantes. Le grec, le roumain (ș ț) et le serbe sont les cas à surveiller.

Critères Go : les 3 polices extraites et vérifiées, police du chat identique à celle de `/ctspike font`.
La couverture n'est pas un critère : elle décide seulement des polices de secours du chat miroir.
Coller la sortie complète (elle se termine par la ligne « Hash des chemins »).

### S5 - chat miroir

Décodeur lancé avec `--wow` et `--flavor` (étape 4). Pour les captures d'écran, utiliser
**Win+Maj+S** (outil Capture de Windows) : la capture de WoW (Impr. écran en jeu) ne voit pas le miroir.

1. **Calage** : `/ctspike miroir calage`, puis `/ctspike miroir echantillon`. Le texte officiel reste
   visible et l'application dessine les mêmes lignes en magenta par-dessus. Le décodeur affiche les
   mesures reçues du jeu (« mesure du jeu », puis « mise en page ChatFrame1 : ... »). Attendu : le
   magenta recouvre exactement le texte officiel, lignes coupées aux mêmes mots. Faire une capture
   (Win+Maj+S) de la fenêtre de chat.
2. **Miroir** : `/ctspike miroir on`. Le texte officiel disparaît, l'application le redessine
   (police du jeu, couleurs, ombre). Écrire `/dire Bonjour à tous`, se chuchoter un message.
   Capture de la fenêtre de chat.
3. **Suivi** : molette dans le chat (les lignes anciennes réapparaissent, puis retour en bas),
   changer d'onglet, déplacer et redimensionner la fenêtre (mode Édition), Alt+Z (interface masquée :
   le miroir disparaît aussi), Alt+Tab vers PowerShell (le miroir se cache, revient au retour).
   Attendre 2 minutes sans écrire : les lignes s'effacent comme d'habitude.
4. **Verrouillage forcé** : `/console addonChatRestrictionsForced 1` : le texte officiel revient aussitôt
   (le décodeur affiche « texte officiel »). Puis `/console addonChatRestrictionsForced 0`.
5. **Démonstration de l'acronyme** : arrêter le décodeur (Ctrl+C), le relancer avec `--demo-trad`,
   puis en jeu `/ctspike chat on` et se chuchoter un message : la ligne affiche `[??]` devant
   le message (la vraie langue viendra avec la détection, S4).
6. `/ctspike miroir off` : tout redevient normal.

Si le décodeur est fermé pendant que le miroir est actif, le chat semble vide : `/ctspike miroir off`
(ou `/reload`) le rétablit. Le miroir n'est jamais réactivé tout seul après un `/reload`.

Critères Go : décalage du calage d'au plus 1 pixel, texte lisible et semblable à WoW, nouvelles
lignes, défilement, onglets, déplacement et effacement suivis sans retard gênant, texte officiel
rétabli sous verrouillage, liens toujours cliquables (lignes non traduites), aucune baisse d'images
par seconde. Le miroir est décalé par défaut de 1 pixel vers la droite (`--dx 1`, réglé au calage
du 08/11/2026) ; si le magenta est encore décalé, relancer le décodeur avec `--dx` et `--dy`
(pixels, ex. `--dx 0 --dy -1`) et noter les valeurs qui alignent.

### S4a - matériel du joueur

Chaque joueur a une machine différente : l'application doit choisir elle-même où traduire
(carte graphique ou processeur) et quelle taille de modèle elle peut porter sans gêner le jeu.
La détection ne charge aucun pilote et ne modifie rien.

1. WoW fermé :

```powershell
cargo run --release --target-dir target-win -- materiel
```

2. WoW lancé, personnage en jeu (de préférence dans une zone chargée) : relancer la même commande.

Attendu : processeur (cœurs, AVX2), mémoire vive totale et libre, chaque carte graphique avec sa
mémoire vidéo dédiée et la part que Windows accorde en ce moment, les moteurs de calcul présents
(Vulkan, CUDA, DirectML, HIP), WoW détecté au 2e passage, puis un niveau proposé (élevé, moyen,
léger, insuffisant) avec la voie choisie et les autres voies possibles. Coller les deux sorties.
Les seuils sont provisoires : S4b les fixera en mesurant vitesse, qualité et images par seconde.

### S4b - comparatif des modèles de traduction

L'outil lance le serveur de llama.cpp avec un modèle, lui fait traduire 60 messages de chat de jeu
(français, anglais, allemand, espagnol, russe, polonais) vers 4 langues chacun : 240 traductions
directes, sans langue intermédiaire, couvrant les 23 langues, dont 200 paires sans l'anglais
(`--court` : 48 ; `--low` : les 30 paires de la version low). Il mesure le chargement, la mémoire,
la latence par message et la vitesse, et écrit chaque traduction dans
`resultats-s4b\<modèle>_<gpu|cpu>[_low]_<réglages>.tsv` pour juger la qualité.

Modèles (fichiers GGUF quantifiés en 4 bits ; place sur le disque telle que Windows l'affiche,
1 Go = 1024³ octets). TranslateGemma 12B est retiré le 09/11/2026 : peu de PC le font tourner à côté
de WoW.

| Modèle | Langues | Disque | Mémoire vidéo chargé | Licence | Rôle |
| --- | --- | --- | --- | --- | --- |
| EuroLLM 1.7B Instruct (UTTER) | version low : 6 | 0,97 Go | 1,1 Go (mesuré) | Apache 2.0 | Low : téléchargé d'office, dernier repli |
| TranslateGemma 4B (Google) | 23 | 2,32 Go | 2,7 Go (mesuré) | conditions Gemma | d'office si l'estimation le conseille, sinon sur demande |
| SalamandraTA 7B v3 (BSC) | 23 | 4,52 Go | 4,3 Go (mesuré) | GPL-3.0 | d'office si l'estimation le conseille, sinon sur demande |
| SalamandraTA 2B (BSC) | version low : 6 | 1,41 Go | - | Apache 2.0 | comparatif seulement |

**Tests lancés depuis WoW** (la façon recommandée) : l'application tourne en arrière-plan
(`tests-en-jeu`), le joueur reste dans WoW et tape `/ctspike essai` dans le chat : la **série
complète** se déroule seule (0.8.6), du plus prudent au plus lourd (pack low sur le processeur,
pack low sur la carte, 4B sur la carte, 4B sur le processeur depuis la 0.8.8, 7B), en sautant les
modèles absents et, après un échec de charge sur la carte, les modèles plus lourds sur la carte
(une mesure incomplète, WoW à l'arrière-plan, n'est pas un échec depuis la 0.8.8). Environ 20 min :
rester dans WoW jusqu'au bilan, sans passer dans une autre fenêtre (le panneau disparaît et
l'addon ne mesure plus quand WoW n'est pas au premier plan). Le bilan s'affiche, est écrit dans `serie-<date>.txt`, et
l'application se ferme seule 60 s après. Un seul test : `/ctspike essai low`, `low-cpu`, `4b`
ou `7b`. L'application lit la demande dans le bandeau de l'addon, lance le test
et affiche son avancement puis son verdict dans un panneau par-dessus le jeu, en haut à droite
(le bandeau est en bas à droite). `/ctspike essai stop` arrête le test, `/ctspike essai fermer`
masque le panneau. WoW reste au premier plan pendant toute la mesure. Une trame du bandeau ne
reste que 50 ms à l'écran : la demande part trois fois avec un numéro, et l'application lit le
bandeau toutes les 5 ms (0.8.5). La capture suit la fenêtre de WoW : le bandeau est lu en bas à
droite de la zone de jeu, en plein écran comme en mode fenêtré, et le panneau s'affiche en haut
à droite de cette zone (0.8.6 ; avant, le coin de l'écran était lu, faux en mode fenêtré). Résultats dans
`%LOCALAPPDATA%\ChevaliersTranslators\resultats-s4b`.

```powershell
$exe = "$ct\ct-phase0\ct-spike\target-win\release\ct-spike.exe"
$jeu = Start-Process $exe -ArgumentList "tests-en-jeu --dossier $m" -WindowStyle Minimized -PassThru
Stop-Process -Id $jeu.Id
```

**Test en jeu depuis PowerShell** (`--fps`, WoW lancé, personnage immobile dans un endroit calme, `/ctspike perf on`,
ne plus toucher à rien jusqu'à la fin, pas d'Alt+Tab). Le test met WoW au premier plan lui-même
(0.8.3), attend que l'addon envoie ses mesures (`/ctspike perf on`, réglage gardé après `/reload`
depuis la 0.8.3), et signale la fin par un bip en ramenant la console devant. Si les mesures
manquent pendant plus de 30 % des traductions (WoW plus au premier plan), il n'y a pas de verdict
en jeu :

1. référence, 15 s sans traduction : images par seconde, et plus longue image de chaque
   demi-seconde (envoyée par l'addon depuis la 0.8) ; le seuil d'à-coup en est tiré
   (au moins 100 ms, ou le double de ce que WoW fait seul) ;
2. chargement du modèle, contrôle de la mémoire vidéo (1 Go doit rester libre) ;
3. 15 s modèle chargé, au repos : la mémoire occupée gêne-t-elle déjà le jeu ?
4. traductions au rythme d'un chat animé (une toutes les 3 s au plus, `--rythme` pour changer),
   **par rafales** et avec le **régulateur** : à-coup au-delà du seuil pendant une traduction,
   la traduction passe un cran plus prudent sans relancer le modèle (priorité minimale sur la
   carte, rafales de 8 puis de 4 jetons, pauses de 80 puis 150 ms, traductions plus espacées) ;
   s'il y en a encore, le test s'arrête de lui-même et indique le réglage suivant à essayer ;
5. lecture des journaux de Windows : toute réinitialisation du pilote graphique pendant le test
   (codes 141 et 117) donne le verdict « a fait planter le jeu ».

Le test s'arrête aussi si WoW se ferme ou si le serveur de traduction tombe. Le verdict est noté
pour ce PC (`%LOCALAPPDATA%\ChevaliersTranslators\essais.tsv`) avec les réglages qui l'ont obtenu ;
l'interface et le premier lancement s'en servent. Chaque seconde (mémoire vidéo libre, part du
serveur et de WoW, images par seconde, plus longue image, traduction en cours) est écrite dans
`resultats-s4b\*_chronologie.tsv`.

**Règle de choix des modèles pour toutes les configurations** : voir `REGLE_MODELES.md`.

**Estimation sans test** (0.8.8) : `ct-spike estimation` lit la carte (bande passante d'après son
nom), le processeur, la mémoire, et les conditions de WoW (dernier test en jeu, sinon
`WTF\Config.wtf` de la version installée, sinon le pire cas), puis affiche pour chaque modèle la
latence et la **capacité** prévues (messages par minute en gardant la baisse sous 5 % ; il en faut
12), son niveau (sûr, probable, limite, non), la **chaîne de repli** et les modèles à télécharger
d'office. Les cartes sont rangées en **classes** de performances théoriques équivalentes (S à F,
d'après la bande passante mémoire) : toutes les cartes d'une classe reçoivent la même prévision,
calculée au bas de la classe ; seule la mémoire vidéo est comptée carte par carte. Seuil de
latence unique de 3 s sur la carte et le processeur (0.8.10). `installation --telecharger` télécharge ces modèles conseillés (`--seulement-low` : le
pack low seul) ; l'interface les marque « Conseillé » et affiche la chaîne. Le verdict des tests
en jeu suit la même règle : une baisse de 5,2 % à 20 messages par minute fait 19 messages par
minute sous 5 %, donc compatible.

```powershell
& $exe estimation
& $exe estimation --wow $wow --flavor _forever_
```

**Rafales** (0.8.1, par défaut sur la carte graphique) : le modèle génère 16 jetons, s'arrête
30 ms pour laisser la carte à WoW, puis reprend là où il en était (le serveur garde le calcul
déjà fait : seule la suite est calculée). Les messages longs sont découpés en phrases. Le joueur
ne voit qu'un message traduit ; la carte n'est jamais occupée longtemps d'affilée. Le régulateur
règle la taille des rafales et les pauses en direct. Depuis la 0.8.7, chaque reprise envoie la
consigne en texte suivie des numéros des jetons déjà générés : le serveur ne recalcule rien
(renvoyer le texte généré le faisait redécouper autrement : 16 à 21 jetons recalculés à chaque
reprise pour EuroLLM et TranslateGemma).

**Mode doux** (par défaut sur la carte graphique, toutes marques) : priorité basse de la
traduction sur la carte (réglage de Windows : WoW passe devant ; appliquée une fois le modèle
chargé, car Windows refuse avant : code 0xc000000d le 09/11/2026), travail envoyé à la carte par
paquets de 10 opérations au lieu de 100 (`GGML_VK_MAX_NODES_PER_SUBMIT`), lots de calcul de 64
(`-ub`). Options pour essayer d'autres réglages :

| Option | Effet |
| --- | --- |
| `--origine` | réglages d'origine de llama.cpp (ceux des plantages du 09/11) |
| `--priorite-gpu normale\|basse\|minimale` | priorité de la traduction sur la carte |
| `--paquets N`, `--lots N` | taille des paquets envoyés à la carte, des lots de calcul |
| `--file-graphique` | partage la file graphique de WoW au lieu d'une file de calcul à part |
| `--couches N` | seulement N couches du modèle sur la carte, le reste sur le processeur |
| `--moteur hip-radeon` | moteur HIP d'AMD au lieu de Vulkan (à télécharger, voir S4c) |
| `--cpu`, `--fils N` | tout sur le processeur (N fils ; par défaut la moitié des cœurs), rien sur la carte |
| `--rafales N`, `--pause MS` | jetons par rafale (16 par défaut), pause entre deux rafales (30 ms) |
| `--sans-rafales` | chaque message d'une traite |
| `--sans-regulateur` | pas de réaction aux à-coups |

Critères : latence médiane sous 3 s, sur la carte comme sur le processeur (seuil unique depuis la
0.8.10 ; 1,5 s sur la carte avant), au moins 12 messages par minute en gardant la baisse moyenne
d'images par seconde sous 5 %, aucune gêne au repos, à-coups sous le seuil, aucune
réinitialisation du pilote ; qualité jugée sur les traductions (sens, ton du chat, abréviations
et noms d'objets conservés).

### S4c - modèles chez le joueur

L'addon publié reste léger (CurseForge accepte 1 Go au plus) : aucun modèle dedans. L'application :

1. télécharge d'office le **pack low** (EuroLLM 1.7B, 0,97 Go : français, anglais, espagnol,
   italien, allemand, russe) et le moteur llama.cpp (0,03 Go) ; le pack low est le modèle par
   défaut et le **repli** ;
2. classe les autres modèles du plus léger au plus lourd en mémoire vidéo ; le **HIGH** d'un
   joueur est le meilleur modèle que son PC fait tourner pendant le jeu ; ils se téléchargent
   **sur demande** ;
3. n'active qu'**un seul modèle** à la fois ; si le modèle choisi n'est plus installé ou a échoué
   au test en jeu, le pack low prend le relais.

Un modèle est proposé sur la carte graphique s'il laisse **au moins 1 Go de mémoire vidéo libre**
pendant le jeu, et s'il n'a pas fait réinitialiser le pilote ni trop baissé les images par seconde
au test en jeu ; sinon, sur le processeur s'il est assez léger (pack low : 4 cœurs ; 4B : 8 cœurs ;
AVX2). WoW est compté pour 7 Go de mémoire vidéo chez tous les joueurs (PC de référence en
3840x1600 : WoW mesuré jusqu'à 6,0 Go en jeu, marge pour les raids), ou pour sa mesure si elle est
plus haute (WoW réduit ou en zone calme en mesure moins : 3,2 Go, ce qui ne dit rien des raids). Mémoire d'un modèle : estimée
(fichier + 15 % + 0,5 Go) jusqu'à sa mesure sur ce PC (`empreintes.tsv`).

Les fichiers déjà présents et complets ne sont pas retéléchargés ; une coupure ou un débit tombé sous
100 Ko/s pendant 30 s reprend là où le téléchargement s'était arrêté ; chaque fichier est vérifié à
l'octet près.

**Interface** : fenêtre de gestion des modèles, fenêtre de l'application elle-même depuis la
0.8.1 (WebView2, le moteur web livré avec Windows 10 et 11 ; à défaut Edge, puis le navigateur). Elle montre la mémoire vidéo de la carte pendant le jeu
(Windows, WoW, modèle, réserve de 1 Go, libre ; survoler un modèle le place dans la jauge), les
modèles avec leur qualité, leur place, la raison pour laquelle ils sont proposés ou non et leur
test en jeu, et permet de télécharger (avec progression et annulation), activer et supprimer.
Le pack low et le moteur se téléchargent d'eux-mêmes à l'ouverture s'ils manquent. Titres dans la
police de WoW (Friz Quadrata, lue dans le cache de `fonts`). Fermer la fenêtre rend la main à
PowerShell, sauf téléchargement en cours (il continue dans la console ; Ctrl+C l'arrête, il
reprendra au lancement suivant).

```powershell
cargo run --release --target-dir target-win -- interface
cargo run --release --target-dir target-win -- interface --demo
```

Pour garder l'interface ouverte et continuer à taper des commandes, la lancer dans sa propre
fenêtre (l'arrêter avant de recompiler : Windows verrouille un programme en cours) :

```powershell
$exe = "$ct\ct-phase0\ct-spike\target-win\release\ct-spike.exe"
$ui = Start-Process $exe -ArgumentList "interface --dossier $m" -WindowStyle Minimized -PassThru
Stop-Process -Id $ui.Id
```

`--demo` : PC de référence simulé, téléchargements simulés, rien n'est écrit.
Version console : `installation` (annonce), `installation --telecharger` (moteur et modèles conseillés, pack low compris ; `--seulement-low` : pack low seul),
`installation --modele translategemma-4b --telecharger` (un modèle sur demande),
`installation --moteur hip-radeon --telecharger` (moteur HIP d'AMD, pour comparer à Vulkan).

**Diagnostic d'un plantage** : `diagnostic --wow $wow --flavor _forever_ --heures 12` lit les
journaux de Windows (réinitialisations du pilote graphique, plantages de WoW et du serveur) et les
rapports d'erreur de WoW (`Errors`). Windows renvoie parfois plus tard des rapports en
attente : une réinitialisation dont le vidage (`WATCHDOG-AAAAMMJJ-HHMM.dmp`) date d'avant la
période examinée est écartée (0.8.3), aussi pendant les tests en jeu. `essai --modele <fichier> --verdict plantage` note à la main
le résultat d'un test fait avant la 0.8.

À vérifier avant la publication : les règles de CurseForge interdisent les liens de téléchargement
externes sur la page d'un projet ; l'application compagnon et le téléchargement des modèles depuis
Hugging Face devront être présentés en conséquence (conditions des auteurs de CurseForge).

### Plantages graphiques du 09/11/2026

Deux plantages de WoW, modèle chargé sur la carte pendant le jeu (12B à 02:22, 7B à 02:47). Les
journaux de Windows montrent des **réinitialisations du pilote graphique** (LiveKernelEvent 141 :
un moteur de la carte n'a pas répondu à temps ; 117 : le pilote a cessé de répondre ; vidages
WATCHDOG, pilote amdkmdag.sys), sans aucune trace de mémoire saturée : avec le 7B il restait
environ 4,8 Go de mémoire vidéo libres. Cause : le temps de calcul de la carte partagé entre WoW et
la traduction. Une réinitialisation ne fait pas toujours planter WoW (deux réinitialisations à
02:45 et 02:46, plantage à 02:47:40), mais Windows n'en tolère que 5 par minute et chacune fige la
carte 2 s : on ne s'en sert pas comme repère. Repère retenu : la plus longue image de WoW, bien
avant les 2 s. Réponse : mode doux, régulateur, détection des réinitialisations, repli sur le
processeur ou le pack low ; ces réglages valent pour toutes les marques de carte.

### S8 - traduction réelle dans le chat (0.9 à 0.11)

Le décodeur charge le modèle choisi par l'estimation de la version du jeu (premier de la chaîne
de repli dont le fichier est installé), puis :

- **démarrage** (0.11, 0.12) : `ct-spike live` réécrit toutes les 2 s un petit addon de présence
  (`CT_Spike_Live`), **dans toutes les versions du jeu installées** (0.12 : lancé pour Forever, il
  sert aussi à retail) ; à chaque connexion et `/reload`, l'addon voit que l'application tourne et
  active la traduction du chat tout seul. **Lancer `ct-spike live` avant d'entrer en jeu** : un addon
  ne relit ses fichiers qu'à la connexion. Application lancée après coup : un panneau sur le jeu
  demande `/reload` (0.12). `/ctspike trad off` coupe la traduction, aussi aux connexions
  suivantes (`/ctspike trad on` pour la remettre). Déconnexion, écran de chargement, fermeture ou
  réduction du jeu : le miroir disparaît. **Une autre application active ne le cache plus** (0.12) :
  le miroir reste au premier plan (WoW en fenêtré plein écran l'est souvent aussi : 0.12.0 le
  mettait dessous, plus rien ne s'affichait) et ne dessine rien sous la fenêtre de l'application
  passée devant WoW (0.12.1) ;
- **tous les messages de joueurs** (0.12) : dire, crier, émote, chuchotements (Battle.net
  compris), groupe, raid, avertissement raid, instance, guilde, officier, Discord de guilde,
  communautés, transcription vocale, messages d'absence, et tous les canaux numérotés (Général,
  Commerce, Services, Défense locale, canaux créés). Chacun garde la couleur choisie dans les
  réglages du chat. Messages du jeu (système, butin, PNJ, émotes standard) : déjà dans la langue du
  client ; langue raciale que le personnage ne parle pas (« [Orc] revas il rath'is ») : charabia du
  jeu, pas traduit (0.12.1) ;
- **règle de traduction** (09/11/2026, 0.12.4) : **tout message dont la langue détectée n'est pas
  celle du client est traduit en entier dans la langue du client.** Une ligne attend sa traduction
  4 s au plus ; si elle n'est pas prête (ou ne peut pas l'être, chat très actif), la ligne s'affiche
  telle quelle et **la traduction la remplace à son arrivée** (jusqu'à 60 s ; avant la 0.12.4,
  elle était abandonnée). Les **acronymes et le jargon du jeu sont traduits par le dictionnaire**
  (`ct-spike/data/termes.tsv`, section `[jargon]`) : « WTS » → « vends », « LFM » → « cherche des
  joueurs », « PST » → « MP », « M+ » → « MM+ », « HC » → « HM », « AH » → « HV », « dps » → « DD »
  en allemand… Les **noms de donjons, raids et lieux prennent le nom du client** (section
  `[noms]`) : donjons et raids de Midnight (Flèche de Coursevent, Terrasse des Magistères,
  Point-nexus Xenas, Flèche du Vide, Faille du Rêve, Marche sur Quel'Danas…), donjons de la saison 1,
  capitales, zones et donjons classiques ; abréviations comprises (« NPX », « PoS », « BRD »),
  article en minuscule au milieu d'une phrase (« pour les Mortemines »). Une forme précédée de « = »
  ne compte qu'écrite exactement ainsi (« =IF » : Forgefer, pas le mot anglais « if ») ;
- **messages reçus et tes messages** (0.11) : chaque message est rangé dans sa langue (23
  langues ; écriture, lettres et mots propres à chaque langue, mémoire de la langue de chaque
  joueur ; le jargon seul, « LF2M tank heal », est neutre). Un message qui peut être dans la langue
  du client l'est : « salut » reste français, même juste après un message roumain. S'il n'est pas
  dans la langue du client, il est traduit et le chat miroir l'affiche à sa place :
  `[2. Commerce] [Hans] : [DE] Je cherche un groupe…`. Même chose pour tes propres messages écrits
  dans une autre langue ; tes envois traduits (Maj+Entrée) s'affichent avec ton texte d'origine :
  `[Hazè] dit : [RO] Salut tout le monde, comment ça va ?`. Un mot propre à une langue suffit
  (0.12 : « hello » seul devient « [EN] Bonjour »). « craft », « recraft » et « ilvl » restent tels
  quels. Liens d'objets, icônes et marques de raid restent intacts ;
- **affichage d'un coup** (0.10) : une ligne à traduire attend sa traduction (4 s au plus) et
  apparaît directement traduite, les plus récentes avec elle ; une ligne dans la langue du client
  s'affiche aussitôt ;
- **lignes laissées à WoW** (0.12.3) : seules les lignes écrites par des joueurs (dire, crier,
  émote, chuchotements, groupe, raid, instance, guilde, officier, communautés, canaux numérotés)
  sont redessinées par l'application. Butin, artisanat, monnaies, expérience, réputation, système,
  erreurs, JcJ, créatures et boss, marques de cible, messages des addons : WoW les dessine
  lui-même, ligne par ligne, à leur place ; ils ne passent plus par le bandeau ni par la console.
  Une ligne est reconnue comme celle d'un joueur par l'événement que la fenêtre de chat range avec
  elle (`CHAT_MSG_SAY`…) ; 0.12.5 : en 0.12.4 le numéro de type ne suffisait pas sur retail, tes
  propres messages restaient dessinés par WoW, non traduits, et un onglet jamais ouvert avant
  l'activation gardait l'ancien mode (tout redessiné, butin compris) ;
  Une traduction plus longue que la ligne d'origine est écrite un peu plus petit pour tenir dans
  la place que WoW lui donne ;
- **chat très actif** (0.12.3) : toute trame de l'addon compte comme signe de vie (le miroir ne
  disparaît plus quand la file du bandeau ne se vide pas) ; capture et décodage dans leur propre
  fil (le dessin du miroir ne fait plus manquer de trames) ; une ligne neuve cite le message relayé
  par son numéro au lieu de le renvoyer (moitié moins de trames) ; les messages des onglets cachés
  (Services, Débutants...) passent après ceux de l'onglet affiché (abandonnés au-delà de 60 s) ;
  un message qui ne serait pas traduit à temps (file pleine) s'affiche tout de suite tel quel au
  lieu d'attendre 4 s, puis sa traduction le remplace ; texte d'une ligne renvoyé toutes les 60 s
  (20 s avant). La console
  signale les trames manquées (`[bandeau]`) et une file qui s'allonge (`[addon] bandeau : …`) ;
  les canaux y portent leur numéro (`[Canal 2]` : Débutants) ;
- **icônes** (0.12.3) : rang de qualité des objets d'artisanat, marques de raid, pièces et autres
  icônes du chat dessinées avec les vraies images du jeu (lues dans ses archives, gardées en cache
  dans `%LOCALAPPDATA%\ChevaliersTranslators\fonts\icones`), à leur taille dans WoW ; une forme
  dessinée les remplace le temps du chargement. Liens d'objets à la couleur de leur qualité
  (`|cnIQ4:` du retail) ;
- **vitesse** (0.10) : jusqu'à 4 messages traduits à la fois ; messages reçus par vagues (60 /
  capacité secondes) ; tes envois passent devant ; **mémoire des phrases**
  (`%LOCALAPPDATA%\ChevaliersTranslators\phrases.tsv`) : une phrase déjà traduite ressert sans
  calcul ;
- **Maj+Entrée** (0.11) : le texte part à la traduction (pendant la frappe, le brouillon est déjà
  traduit), la boîte se rouvre vide sur le même canal, la traduction y est collée (Ctrl+V) puis
  envoyée (Entrée) : deux touches simulées par l'application, avec ton accord, seulement à ce
  moment-là, seulement si WoW est au premier plan et la boîte prête. Ton presse-papiers est remis
  comme avant. Plus aucun emplacement consommé, plus de `/reload` ;
- **bouton Trad** (0.11, 0.12) : à part, à droite de la barre de saisie, visible même saisie
  fermée ; un clic ouvre le menu, un clic sur la langue la choisit (un seul niveau :
  Automatique et les six langues principales en tête, puis les autres par région). Clic droit :
  Automatique. `Trad:AUTO` : langue du destinataire d'un chuchotement, sinon langue
  **majoritaire** des messages du canal des 15 dernières minutes (0.12 : chaque canal numéroté à
  part, Commerce n'est pas Général ; les plus récents comptent plus ;
  tes phrases écrites dans une autre langue comptent aussi) — canal surtout français : rien n'est
  traduit —, sinon l'anglais. Retour à Automatique 60 s après un choix à la main et un envoi ;
- **menus et bulles au-dessus du miroir** (0.11, 0.12) : bulles d'aide (liens d'objets), menus
  (onglets du chat, bouton Trad), réglages du chat, dialogues, sacs et fenêtres du jeu posés sur
  le chat : le miroir ne dessine rien dessous, et seulement dessous (0.12 : la zone exacte du menu ;
  un sous-menu est ajouté quand la souris y passe ; 0.12.2 : 10 px de marge pour les bordures) ;
- **copier un message** (0.11) : une petite icône au bout de chaque message de joueur ; un clic
  copie le message tel qu'il s'affiche (traduit, sans l'heure, le canal ni le nom) ; une coche verte
  le confirme. WoW garde le clavier ;
- **repli en jeu** : serveur arrêté, mémoire insuffisante, ou latence médiane au-delà de 3 s :
  l'échec est noté (`essais.tsv`) et l'étape suivante de la chaîne prend le relais.

```powershell
& $exe live --wow $wow --flavor _forever_ --dossier $m
& $exe live --wow $wow --flavor _retail_ --dossier $m
```

`--sans-mes-messages` : tes messages restent tels quels. La console affiche la langue de chaque
message, la traduction et son temps, les envois (« collé et envoyé »), les copies, l'état du
modèle. Journal du serveur : `%LOCALAPPDATA%\ChevaliersTranslators\traduction-serveur.log`.

Critères Go : messages étrangers directement traduits, avec l'acronyme ; messages dans la langue
du client (« salut ») et jargon tels quels ; Maj+Entrée une seule fois sur tous les canaux ; menus
du chat lisibles ; copie d'un message ; traduction active à la connexion, miroir disparu à la
déconnexion.

### S9 - retail (0.11)

1. **Canaux absents du miroir** : sur retail, traduction active, attendre quelques messages du
   canal Services, puis `/ctspike diag`. La console de `ct-spike live` recopie les lignes
   `[addon] diag : …` (fenêtres de chat, lignes visibles, lignes « secrètes », messages reçus par
   type) : les envoyer, elles disent pourquoi ces lignes ne s'affichent pas.
2. **Tests de performance** : retail est estimé plus lourd que Forever (10 Go de mémoire vidéo
   pour WoW au lieu de 7 ; `REGLE_MODELES.md`, version 2.3 ; 12 Go mesurés depuis la version 2.4). `ct-spike estimation --wow $wow
   --flavor _retail_` montre la chaîne prévue. Puis la série complète **dans retail**, réglages au
   maximum : `ct-spike tests-en-jeu --dossier $m`, et en jeu `/ctspike essai` (environ 20 min, sans
   bouger). Chaque test note la version du jeu (« version=_retail_ ») : retail a désormais ses
   propres résultats, l'estimation de Forever ne change pas.

### S7 - coexistence avec QuestVoice

QuestVoice et son application lancés, client français. Pendant `/ctspike test 2000 20`,
parler à des PNJ et ouvrir des quêtes : QuestVoice doit lire toutes les quêtes, et le décodeur
ne doit perdre aucune trame. `/ctspike info` indique si QuestVoice est détecté.

### Clôture de la phase 0 (0.12.5) - bilan : `BILAN_PHASE0.md`

Trois passages dans retail, réglages au maximum, plein écran 3840x1600, puis le bilan go/no-go.

**1. S8 - traduction dans le chat** (`ct-spike live`) : `/dire hello` doit s'afficher
`[Hazè] dit : [EN] bonjour` ; un message de joueur étranger reçu, traduit à sa place ; butin,
système et PNJ affichés par WoW (absents de la console) ; l'icône de rang de qualité d'un objet
d'artisanat dessinée avec sa vraie image.

**2. S9 - série de performances retail** (`ct-spike tests-en-jeu`) : `ct-spike live` arrêté
depuis au moins 20 s, puis `/reload` (la traduction reste coupée : rien d'autre ne charge le
bandeau ni la carte graphique pendant les mesures). `/ctspike essai`, WoW au premier plan, sans
bouger, environ 20 min ; l'application se ferme seule et écrit le bilan.

**3. S7 - QuestVoice** (`ct-spike live` + QuestVoice et son application) : `/ctspike info`
(QuestVoice détecté), `/ctspike test 2000 20` en parlant à des PNJ et en ouvrant des quêtes, puis
10 min de jeu normal (quêtes et chat) ; Ctrl+C sur `ct-spike live` donne le résumé.

**Critères du go** (le bilan est écrit d'après ces critères, pas d'impression) :

| Point | Go si |
| --- | --- |
| S1 avec QuestVoice | 2000/2000 trames, aucune illisible |
| S7 | QuestVoice lit toutes les quêtes ; aucune ligne `[bandeau] … trame(s) manquée(s)` ni « Trames du bandeau manquées » au résumé ; son bandeau (en haut à gauche) jamais recouvert |
| S8 | les quatre vérifications du point 1 |
| S9 | au moins un modèle « sûr » ou « probable » dans retail au maximum : latence médiane ≤ 3 s, au moins 12 messages par minute avec une baisse d'images/s sous 5 %, à-coups sous 100 ms (ou le double de WoW seul), aucune réinitialisation du pilote (`REGLE_MODELES.md`) |

Un critère manqué n'est pas forcément un no-go : le bilan dit s'il se corrige dans le produit
(phase 1) ou s'il remet en cause le principe (bandeau de pixels, miroir, traduction locale).

## Résultats

| Date | Version | Écran | Test | Résultat |
| --- | --- | --- | --- | --- |
| 08/11/2026 | Forever 1.60.1.70862 (projet 18) | 3840x1600 | S1, 1000 trames à 20/s, QuestVoice chargé | 1000/1000, 0 illisible, 0 contenu faux ; 15,3 trames/s (cadence arrondie, corrigée en 0.1.1) |
| 08/11/2026 | Forever 1.60.1.70862 | - | S6, emplacement 1 réécrit 6 min après la connexion | Jeton relu (3968ED) en 2,3 ms |
| 08/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S1 v0.1.1, 1000 trames à 20/s | 1000/1000, 20,0 trames/s, capacité 5104 octets/s |
| 08/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S1 v0.1.1, 2000 trames à 30/s | 2000/2000, 22,5 trames/s (plafond : 2 images du jeu par trame), capacité 5740 octets/s |
| 08/11/2026 | Forever 1.60.1.70862 | - | S2, chuchotements | Reçu et envoyé relayés et affichés par le décodeur |
| 08/11/2026 | Forever 1.60.1.70862 | - | S6 envoi | /dire envoyé depuis un emplacement ; chuchotement raté (nom sans le nom de famille), langue non précisée : corrigés en 0.1.2 |
| 08/11/2026 | Forever 1.60.1.70862 | - | S6 v0.1.2 | 4/4 jetons justes ; chuchotement reçu sous le nom complet ; /dire dans la même langue que la frappe manuelle ; GetUnitName et UnitFullName donnent le nom complet |
| 08/11/2026 | Forever 1.60.1.70862 | - | S3 v0.2.0 | Entrée intacte (aussi sous verrouillage forcé) ; Maj+Entrée intercepté sans envoi (aussi sous verrouillage) ; 2 chuchotements traduits reçus ; envoi par addon refusé sous verrouillage avec renvoi vers Ctrl+V ; traduction prête avant le 2e appui (aucune réaffectation nécessaire) |
| 08/11/2026 | Forever 1.60.1.70862 | - | S5a v0.3.0 | Archives lues en 0,7 s (racine MFST v2, 2 433 304 entrées dont 231 044 nommées) ; 3 polices trouvées par leur nom, MD5 vérifiés. ARIALN.TTF (FDID 615958, « Arial Narrow », 661 caractères) : 22 langues sur 23, manque ș ț roumains ; FRIZQT__ : latin occidental seul ; FRIZQT___CYR : + cyrillique. Hash des chemins : ordre c:b |
| 08/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S5 v0.4.0 | Miroir affiché par-dessus WoW, police du jeu chargée depuis le cache. Calage : hauteur et position verticale des lignes justes ; texte du miroir 7 % plus étroit que celui de WoW (495 px contre 531 px sur une même ligne) et lignes coupées sans le retrait de WoW (environ 15 px) : corrigé en 0.4.1 par les mesures faites dans le jeu. Déplacement et redimensionnement suivis en direct ; verrouillage forcé : texte officiel rétabli, puis miroir repris |
| 08/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S5 v0.4.1 | Calage : lignes coupées aux mêmes mots, même retrait, magenta posé sur le texte officiel ; seul écart, tout le texte 1 px trop à gauche (décalage par défaut en 0.4.2). Molette, onglets, effacement suivis ; 44 images/s constantes avec le miroir. Roumain ș ț : carrés dans WoW, lettres justes dans le miroir (espacement corrigé en 0.4.2). Icône de cible : carré gris dans le miroir. Crénage appliqué par WoW : To, Yo, P. à -1 px, AV à 0. Retrait non mesuré par la sonde (lu sur les vraies lignes en 0.4.2), estimation juste |
| 08/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S5 v0.4.2 | Calage au pixel : 199 pixels de texte officiel non recouverts (contre 5 301 en 0.4.1), sans écart dans un sens ou l'autre. Miroir caché quand WoW n'est plus au premier plan (Alt+Tab). Crénage mesuré : To, Yo, P., Te, Ye, Ta à -1 px ; avances de WoW 8,3 % plus larges que celles de la police brute. Retrait des lignes coupées toujours estimé (16 px, juste) : nouvel essai de mesure en 0.5.0 |
| 08/11/2026 | Forever 1.60.1.70862 | - | S4a v0.5.0 | Ryzen 7 5800X3D (8 cœurs, AVX2), 64 Go de mémoire vive, Radeon RX 6950 XT 16 Go ; Vulkan, DirectML, HIP et OpenCL présents ; niveau proposé élevé, sur la carte graphique (Vulkan). La part accordée par Windows reste à 15,2 Go avec WoW lancé : elle ne compte pas les autres programmes, d'où la mesure de la mémoire réellement utilisée en 0.6.0. WowVoiceProxy pris pour le jeu : exclu en 0.6.0 |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4c v0.7.0 | Annonce juste, WoW lancé : pack low sur la carte graphique, TranslateGemma 12B retenu (7,8 Go de mémoire vidéo libres pendant le jeu pour 7,5 Go nécessaires), 7,81 Go à télécharger, 128 Go libres. Moteur llama.cpp b12211 (Vulkan) et pack low téléchargés et vérifiés (environ 100 Mo/s) ; 12B coupé par le serveur à 0,9 Go (débit tombé, puis connexion fermée) : reprise automatique ajoutée en 0.7.1 |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S4c v0.7.1 | Plantage graphique avec TranslateGemma 12B chargé pendant le jeu (marge prévue : 0,3 Go). Règle retenue : au moins 1 Go de mémoire vidéo libre une fois le modèle chargé ; empreinte des modèles mesurée et gardée, garde-fous dans le comparatif, contexte réduit à 1024 jetons (0.7.2). Avec cette règle et l'estimation, ce PC reçoit SalamandraTA 7B en jeu, tant que la mesure du 12B ne montre pas qu'il tient |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S4b v0.7.2 | SalamandraTA 7B, 48 traductions en jeu : latence médiane 1,05 s, 4,3 Go de mémoire vidéo (estimé 5,7), 42,4 images/s sans traduction et 34,5 pendant (baisse 18,6 %), puis plantage de WoW. Journaux de Windows : 3 réinitialisations du pilote graphique (codes 141 et 117, amdkmdag.sys) et un rapport de délai d'AMD ; même cause pour le plantage du 12B (2 réinitialisations). Pas de manque de mémoire (≈ 4,8 Go libres). Qualité : noms de donjons et de rôles traduits mot à mot (Deadmines en « Dead Cops », tank en « δεξαμενή », hil en « chico ») : dictionnaire des termes du jeu indispensable. 12B retiré ; mode doux, régulateur et détection des réinitialisations en 0.8.0 |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4c v0.8.0 | 49 tests réussis ; interface ouverte dans Edge, TranslateGemma 4B téléchargé depuis l'interface et prêt ; la console restait occupée après la fermeture (Edge ne prévient pas l'outil) : fenêtre propre à l'application en 0.8.1. Idée retenue : traduire par rafales courtes avec pauses pour garder les gros modèles sur la carte (0.8.1) |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S4b v0.8.0 | Pack low (EuroLLM 1.7B) en jeu, mode doux sans rafales. Sur le processeur (4 fils) : 42,6 images/s sans traduction, baisse moyenne 0,9 %, plus longue image 45 ms, latence médiane 0,82 s : compatible. Sur la carte : baisse 1,5 %, plus longue image 78 ms (seuil 100 ms), latence 0,63 s, 1,0 Go de mémoire vidéo, aucune réinitialisation : compatible. Priorité basse refusée par Windows (0xc000000d) juste après le lancement : appliquée après chargement en 0.8.2. Qualité d'EuroLLM faible (étiquettes « Francés: », « Translation: » devant la réponse, tank en « réservoir », Deadmines en « mines de charbon ») : étiquettes retirées en 0.8.2, choix du pack low à revoir au comparatif |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4b v0.8.0 | TranslateGemma 4B et SalamandraTA 7B (mode doux, sans rafales) : aucune mesure de l'addon, WoW à 3,2 Go de mémoire vidéo (pas au premier plan) ; pas de réinitialisation, mais la carte n'était pas partagée avec un WoW en rendu : pas de verdict en jeu. Latence médiane 0,71 s (4B) et 0,79 s (7B). Le zip de la 0.8.1 n'a pas été pris (téléchargement enregistré sous un autre nom) : zips nommés avec la version à partir de la 0.8.2 |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4b v0.8.2 | Interface lancée à part (`Start-Process`) : commandes possibles pendant qu'elle tourne. Tests 4B et 7B : aucune mesure de l'addon en 2 min (mesures non gardées après `/reload`, ou WoW pas au premier plan) ; diagnostic d'une heure : la réinitialisation de 02:47 signalée à nouveau par Windows. En 0.8.3 : `perf on` gardé, WoW mis au premier plan par le test, bip de fin, anciennes réinitialisations écartées |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4b v0.8.3 | Tests en jeu lancés depuis PowerShell jugés impraticables (aller-retour entre la console et WoW, ne rien toucher pendant 4 min). En 0.8.4 : tests lancés depuis le chat de WoW (`/ctspike essai 4b`), suivis et lus dans un panneau par-dessus le jeu. Correction : le bandeau de l'addon est en bas à droite (les conseils de la 0.8.3 disaient en haut à gauche) |
| 09/11/2026 | Forever 1.60.1.70862 | - | S4b v0.8.4 | 55 tests réussis, application des tests en jeu lancée ; aucune demande reçue de WoW. Cause probable : l'application lisait le bandeau toutes les 50 ms alors qu'une trame n'y reste que 50 ms. En 0.8.5 : lecture toutes les 5 ms dans un fil à part, demande envoyée trois fois avec un numéro, panneau d'accueil compté à partir de son apparition au-dessus de WoW |
| 09/11/2026 | Forever 1.60.1.70862 | fenêtré | S4b v0.8.5 | Demande : une seule commande pour toute la série, fermeture automatique à la fin (fait en 0.8.6). WoW est en mode fenêtré pour les tests : la capture lisait le coin bas-droit de l'écran, pas celui de la fenêtre de WoW, ce qui explique les mesures et demandes jamais reçues depuis la 0.8.2 ; en 0.8.6, la capture suit la fenêtre de WoW |
| 09/11/2026 | Forever 1.60.1.70862 | fenêtré, 75 images/s plafonnées | S4b v0.8.6 | Série complète lancée par `/ctspike essai`, déroulée et fermée seule. Les 4 tests compatibles, aucune réinitialisation du pilote, régulateur resté au départ. EuroLLM processeur : 0,85 s, baisse 0 %, plus longue image 26 ms. EuroLLM carte : 0,50 s, 0 %, 35 ms, 1,0 Go. TranslateGemma 4B : 1,22 s, 0 %, 19 ms, 2,5 Go. SalamandraTA 7B par rafales : 1,09 s, baisse 0,7 %, 21 ms, 4,2 Go. Réserve : WoW fenêtré plafonné à 75 images/s et à 3,4 Go de mémoire vidéo, la carte avait de la marge ; les plantages étaient en plein écran 3840x1600 à 42 images/s (carte saturée) : à refaire dans ces conditions. Les rafales ralentissent le 4B (chaque reprise relit 21 jetons au lieu de 1 pour le 7B) : reprendre à partir des numéros de jetons exacts |
| 09/11/2026 | Forever 1.60.1.70862 | 1080p plein écran (série) / 3840x1600 (avant) | S4b extrapolation | La série v0.8.6 était en plein écran 1080p (WoW plafonné à 75 images/s, 3,4 Go). Le 7B génère un jeton en 38 ms aussi bien en 1080p qu'en 3840x1600 (31,8 ms WoW à l'arrêt) : la carte était aussi partagée qu'en grand écran, et le mode doux avec rafales n'a provoqué aucune réinitialisation. Les images par seconde, elles, ne baissent pas en 1080p parce que WoW plafonné laisse du temps libre à la carte ; en 3840x1600 WoW occupe toute la carte (42 images/s). Modèle : la baisse suit la quantité de modèle lue par seconde (chaque jeton relit tout le modèle), étalonné sur les deux mesures en grand écran (7B en continu −18,6 %, pack low au rythme du chat −1,5 %, prévu −2,0 %). Estimation en 3840x1600, une traduction toutes les 3 s : pack low −1,5 % (mesuré), 4B ≈ −3 à −4 %, 7B ≈ −5 à −6 % en moyenne et ≈ −18 % pendant chaque traduction. À confirmer par un seul test du 7B en grand écran |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 plein écran, qualité maximale | S4b v0.8.6 | Série en grand écran (41,8 images/s, WoW 6,0 Go) : pack low processeur 0,84 s / −0,7 % ; pack low carte 0,76 s / −1,5 % ; 4B 1,71 s / −4,8 %, refusé pour la latence (seuil 1,5 s) ; 7B sauté. Aucune réinitialisation. La latence du 4B venait en partie des rafales (21 jetons recalculés à chaque reprise) : reprise exacte en 0.8.7 ; un échec de latence seule ne fait plus sauter les modèles plus gros (le 7B est plus rapide que le 4B) ; chaque test note l'écran, les images par seconde de référence et le plafond. Règle générale : `REGLE_MODELES.md` |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 plein écran, qualité maximale | S4b v0.8.7 | Reprise exacte des rafales : pack low processeur 0,83 s / −0,7 % ; pack low carte 0,65 s (0,76 avant) / −1,6 % ; 4B 1,57 s (1,71 avant) / −5,2 %, mesure en jeu incomplète (63 % : WoW à l'arrière-plan pendant une partie du test) ; 7B encore sauté (la série prenait la mesure incomplète pour un échec : corrigé en 0.8.8). Le 4B recalcule toujours 21 jetons par reprise (fenêtre glissante de Gemma) : `--swa-full` en 0.8.8. Règle v2 : estimation sans test, capacité en messages par minute, chaîne de repli et téléchargements conseillés |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 plein écran, qualité maximale | S4b v0.8.8 | Série complète en 5 tests, tous compatibles, mesures en jeu complètes, aucune réinitialisation, régulateur resté au départ. Pack low processeur 0,95 s / −0,5 % ; pack low carte 0,69 s / −1,6 % / 1,1 Go ; 4B carte 1,42 s / −4,1 % / 2,7 Go (24 messages par minute sous 5 %) ; 4B processeur 2,88 s, aucune baisse mesurable ; 7B carte 1,27 s / −6,1 % / 4,3 Go, plus longue image 62 ms (16 messages par minute sous 5 %). `--swa-full` vérifié : le 4B ne relit plus qu'un jeton par reprise. Prévisions de la 0.8.8 à 14 % près ; valeurs de référence et empreintes mises à jour en 0.8.9 |
| 09/11/2026 | - | - | règle v2.2, v0.8.10 | Seuil de latence unique de 3 s (carte et processeur), décidé après la série : la gêne pour le jeu est comptée à part (messages par minute sous 5 %). Cartes rangées en 7 classes de performances théoriques équivalentes (S à F, bande passante mémoire), même prévision dans une classe, calculée au bas de la classe ; processeurs par classe de cœurs. Effet : les cartes de classe C et D avec 12 Go (RTX 4070 Super, RTX 3060 12 Go, RX 6700 XT, Arc B580) passent au 4B (23 langues). Un ancien « trop lourd » est rejugé avec les nouveaux seuils, sauf à-coups |
| 09/11/2026 | - | - | S8 v0.9.0 | Traduction réelle branchée (pas encore essayée en jeu). Détection de la langue : 138 messages de chat écrits pour le test dans les 23 langues tous reconnus (le détecteur `whatlang` seul : 123), et les 65 messages du banc ; jargon seul classé neutre. Liens, icônes, noms de donjons et jargon protégés par repères numérotés ; cache des messages répétés ; espacement selon la capacité ; repli en jeu sur plantage, mémoire et lenteur |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600 plein écran, qualité maximale, échelle de rendu 200 % | S9 série v0.12.5 | Retail mesuré : 41,9 images/s sans traduction, WoW jusqu'à 11,7 Go, 2,9 Go libres sans modèle. Pack low carte 0,39 s / −1,9 % / 53 par minute (sûr) ; pack low processeur 0,92 s / −8,1 % / 12 par minute (juste au seuil) ; 4B carte et 7B carte impossibles (mémoire) ; 4B processeur 2,98 s / −15 % / 6 par minute (trop lourd). Aucune réinitialisation, à-coups sous 55 ms. En retail au maximum, seul le pack low tient. Le 4B avait été chargé sur la carte par `live` avec 0,2 Go de marge réelle : en 0.12.6, WoW retail compté 12 Go et mémoire vidéo surveillée pendant le jeu (règle 2.4) |
| 10/11/2026 | Retail 12.1.5.70783 | 3840x1600 | S8 v0.12.5 en jeu | Validé : « hello » affiché « [EN] bonjour », messages étrangers traduits à leur place, butin, système et PNJ affichés par WoW, icône de rang de qualité dessinée |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600 | S8 v0.12.4 en jeu | 93/93 tests ; « hello » traduit en console (« bonjour ») mais la ligne restait affichée par WoW en anglais : lignes des joueurs prises pour des lignes du jeu (numéros de type identiques sur retail) ; onglet 5 encore entièrement redessiné (fenêtre sans lignes au premier passage). Corrigé en 0.12.5 (événement de la ligne) |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600 | S8 v0.12.2 en jeu (raid) | 84/84 tests ; traduction active à la connexion, Commerce et Services traduits, « hey sexy ladies » → « bonnes filles » (0,6 s). À régler (fait en 0.12.3) : Commerce lent à arriver, Services en retard sous le spam (signe de vie perdu quand la file du bandeau ne se vide pas) ; icônes de rang de qualité en carré gris ; messages du canal Débutants visibles en console mais pas dans l'onglet affiché (canal absent de l'onglet) ; butin, artisanat et système redessinés par l'application (à laisser à WoW) |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600 | S8 v0.12.1 en jeu | Chat redessiné ; message de guilde du joueur en anglais affiché « [EN] bonjour à tous » avec l'icône de copie. À régler (fait en 0.12.2) : bas du menu de l'onglet encore recouvert ; « Envoi traduit annulé » affiché alors que l'envoi était parti (le collage déclenchait l'annulation) |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600 | S8 v0.12.0 en jeu | 84/84 tests ; **traduction active dès l'arrivée en jeu, sans commande** (« Traduction du chat active (ct-spike live 0.12.0) ») ; 4B sur la carte cette fois ; « hello » envoyé avec Entrée traduit (« [trad EN → FR, 0,3 s] bonjour »). À régler (fait en 0.12.1) : plus rien d'affiché à la place du chat (miroir passé sous WoW, qui est lui-même « toujours au premier plan ») ; « [Orc] revas il rath'is » relayé |
| 09/11/2026 | Retail 12.1.5.70783 | 3840x1600, échelle de rendu 200 % | S8 v0.11.0 en jeu | 84/84 tests ; 4B sur le processeur choisi (3,0 Go de mémoire vidéo libres avec retail au maximum) ; envois en un appui collés et partis (« Hello », « hello, little one ») ; copie d'un message réussie ; canaux Commerce traduits dans le miroir (« Hello need craft » en 3,1 s). À régler (fait en 0.12) : traduction pas lancée seule (application lancée pour Forever, puis après la connexion) ; trou du menu du chat trop large ; miroir caché dès qu'une autre application est active ; bouton Trad dans la barre de saisie ; « hello » écrit par le joueur pas traduit ; console envahie par les lignes renvoyées toutes les 20 s |
| 09/11/2026 | Forever 1.60.1.70862 | 3840x1600 | S8 v0.10.0 en jeu | 79/79 tests ; addon installé dans les 5 versions (classic era, Forever, PTR, retail, XPTR) et vu à jour ; 7B chargé (étape 1 sur 5). Brouillons traduits en 0,1 à 0,4 s ; messages roumains reçus traduits (« [RO] Salut tout le monde, comment ça va ? »). À régler (fait en 0.11) : envois ratés (« emplacement consommé trop tôt » 8 fois de suite, emplacements épuisés en 4 min) ; « salut » affiché « [RO] Salut ! » ; Trad:AUTO resté sur le roumain sur un canal français ; menus du chat sous le miroir ; menu Trad en sous-menus ; traduction à activer à la main ; retail : canaux absents du miroir. Demandés : icône de copie, tests de performance sur retail |
| 09/11/2026 | Forever 1.60.1.70862 | - | S8 v0.9.0 en jeu | Traduction réelle vue dans le chat : propres messages allemands traduits en français (« [DE] Salut tout le monde, quelqu'un a-t-il envie d'un donjon ? »), « LF2M tank heal » laissé tel quel, lien d'objet gardé dans « [EN] WTS,[Chemise aérienne de cherchevent], murmure-moi des offres », messages anglais du canal Général traduits, Trad:DE vers l'allemand. À régler (fait en 0.10) : ligne d'abord affichée en anglais puis remplacée ; deux appuis sur Maj+Entrée ; ligne de l'addon « Envoyé sur… » remplacée aussi ; quelques messages anglais courts non reconnus (« but i personally like it ») ; demandés : menu de langue, retour à Automatique, miroir caché à la déconnexion, installation dans toutes les versions, phrases gardées d'une partie à l'autre |

## Commandes de l'addon

`/ctspike info` · `test [n] [hz]` · `stop` · `ping [texte]` · `chat on|off` · `block [px]` ·
`rate [hz]` · `slot [whisper|say|guild|party|raid|instance] [cible]` · `lang [xx]` · `font` ·
`trad on|off` (traduction du chat, active d'office à la connexion quand `ct-spike live` tourne ; `off` gardé) · `diag` (fenêtres et messages, recopié dans la console) · `lang auto|xx` (langue de Maj+Entrée, aussi le menu du bouton Trad) ·
`miroir on|calage|off|sync|echantillon` · `perf on|off` (images par seconde et plus longue image, gardé après `/reload`) · `essai` (série complète de tests depuis le jeu) · `essai low|low-cpu|4b|4b-cpu|7b|stop|fermer|aide`

Maj+Entrée dans la boîte de saisie : envoi traduit (0.11 : plus de raccourci boîte fermée).
