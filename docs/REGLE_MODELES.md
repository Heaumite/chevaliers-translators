# Chevaliers Translators - règle de choix du modèle de traduction

Version 2.4 du 10/11/2026 (application 0.12.6), établie sur les tests en jeu de la phase 0
(v0.7.2 à v0.8.8 sur WoW Forever, série retail du 09/11/2026 en 0.12.5) ; la version 2.3 avait
ajouté retail sur estimation, la version 2.4 le remplace par la mesure et surveille la mémoire
vidéo pendant le jeu. Elle décide pour n'importe quel joueur, **sans test chez lui** :

- quel modèle proposer, et où le faire tourner (carte graphique ou processeur) ;
- quels modèles télécharger dès l'installation ;
- dans quel ordre se replier en jeu si le meilleur échoue.

Elle est appliquée par `ct-spike estimation` (code : `ct-spike/src/plan.rs`), par l'installation
et par l'interface.

## 1. Ce que disent les mesures

PC de référence : Ryzen 7 5800X3D (8 cœurs), 64 Go de mémoire vive, Radeon RX 6950 XT 16 Go
(bande passante mémoire de 576 Go/s), WoW Forever en **qualité graphique maximale** avec les
modèles de personnages récents. Rythme du test : une traduction toutes les 3 s (20 par minute).
Sur la carte : mode doux et rafales de 16 jetons.

| Modèle | 3840x1600 plein écran, 40 à 42 images/s, non plafonné (0.8.8) | 1080p, plafonné à 75 images/s (0.8.6) |
| --- | --- | --- |
| EuroLLM 1.7B (pack low), processeur, 4 fils | 0,95 s ; −0,5 % ; plus longue image 49 ms | 0,85 s ; 0 % ; 26 ms |
| EuroLLM 1.7B, carte | 0,69 s ; −1,6 % ; 51 ms ; 1,1 Go | 0,50 s ; 0 % ; 35 ms |
| TranslateGemma 4B, carte | 1,42 s ; −4,1 % ; 53 ms ; 2,7 Go | 1,22 s ; 0 à −2 % ; 19 ms |
| TranslateGemma 4B, processeur, 4 fils | 2,88 s ; aucune baisse mesurable ; 48 ms | - |
| SalamandraTA 7B, carte | 1,27 s ; −6,1 % ; 62 ms ; 4,3 Go | 1,06 s ; −0,7 à −4,1 % ; 25 ms ; 4,2 Go |

Valeurs : latence médiane ; baisse moyenne d'images par seconde ; plus longue image pendant les
traductions ; mémoire vidéo du modèle. Aucune réinitialisation du pilote graphique dans aucun de
ces tests.

Les 5 tests de la série du 09/11/2026 en 0.8.8 sont compatibles, mesures en jeu complètes. Les
prévisions faites avant ce test s'en écartaient de 14 % au plus :

| Test | Prévu (0.8.8) | Mesuré |
| --- | --- | --- |
| Pack low, processeur | 0,83 s ; 143 messages par minute | 0,95 s ; 181 |
| Pack low, carte | 0,65 s ; 63 | 0,69 s ; 61 |
| 4B, carte | 1,35 s ; 19 | 1,42 s ; 24 |
| 4B, processeur | 3,10 s ; 59 | 2,88 s ; sans baisse mesurable |
| 7B, carte | 1,15 s ; 14 | 1,27 s ; 16 |

Plantages : SalamandraTA 7B et TranslateGemma 12B avec les **réglages d'origine** de llama.cpp
(gros envois de calcul, traductions enchaînées sans pause) en 3840x1600 : 3 réinitialisations du
pilote (codes 141 et 117), WoW arrêté, −18,6 % d'images par seconde. Jamais de manque de mémoire.

## 2. Ce que coûte une traduction

- **Mémoire vidéo** : l'empreinte du modèle chargé, fixe. Mesurée sur le PC de référence avec les
  4 contextes de la 0.8.8 : 1,1 / 2,7 / 4,3 Go. L'estimation tirée de la taille du fichier comptait
  0,6 à 1,5 Go de trop : elle ne sert plus que pour un modèle inconnu.
- **Charge sur la carte** : chaque jeton relit tout le modèle en mémoire. Quand WoW occupe la
  carte, la baisse d'images par seconde suit le débit de modèle (taille du modèle × jetons par
  seconde) rapporté à la bande passante de la carte :

  > baisse ≈ γ × débit de modèle / bande passante, avec γ ≈ 1,0 à 1,2

  Elle est donc **proportionnelle au nombre de messages traduits par minute**.
- **Capacité** : nombre de messages par minute traduits en gardant la baisse sous 5 %.
  > capacité = 20 × 5 % / baisse mesurée à 20 messages par minute

  PC de référence en 3840x1600 non plafonné : pack low sur la carte 61 par minute, sur le
  processeur 181, 4B sur la carte 24, 4B sur le processeur sans limite mesurable, 7B 16.
- **WoW plafonné** (synchronisation verticale, limite d'images, écran plus petit) : la carte finit
  chaque image en avance et la traduction prend ce temps libre. Au-dessous de 60 % d'occupation,
  il ne reste qu'environ un tiers de la baisse (1080p plafonné à 75 : 7B −0,7 à −4,1 % au lieu de
  −7 %).
- **Latence** : nombre de jetons × temps par jeton, plus la lecture de la consigne. Le partage de la
  carte avec WoW allonge le temps par jeton (4B : ×2 en 3840x1600, ×1,6 en 1080p ; 7B : ×1,2). Elle
  ne suit pas la taille : le 7B est plus rapide que le 4B sur ce PC.
- **Plantages** : liés aux gros envois de calcul ininterrompus, pas à la mémoire. Le mode doux
  (priorité basse, paquets de 10 opérations, lots de 64) et les rafales les ont fait disparaître.

## 3. L'estimation, sans test

### Ce qui est lu sur le PC

- **Carte** : son nom (donné par Windows) donne sa bande passante mémoire dans une table d'environ
  80 cartes NVIDIA, AMD et Intel. Carte inconnue : 28 Go/s par Go de mémoire vidéo. Carte portable :
  ×0,75. Intel Arc : ×0,7 (llama.cpp y est moins rapide). Cette bande passante utile range la carte
  dans une **classe** (tableau ci-dessous). Mémoire vidéo dédiée et utilisée.
- **Processeur** : cœurs, AVX2, rangé lui aussi par classe (4, 6, 8, 12, 16 cœurs et plus) ;
  **mémoire vive** libre.
- **Conditions de WoW**, dans cet ordre :
  1. celles du dernier test en jeu (taille de l'écran, images par seconde, plafond), si l'écran
     n'a pas changé depuis ;
  2. sinon les réglages de WoW (`WTF\Config.wtf` de la version installée) : résolution, échelle de
     rendu, qualité graphique, limite d'images (`maxFPS`), synchronisation verticale ;
  3. sinon le pire cas : WoW occupe toute la carte (qualité maximale, sans plafond).

### Classes de cartes

Les cartes de performances théoriques équivalentes reçoivent la même prévision : les calculs
prennent la bande passante du **bas de leur classe**, prudente pour les cartes du haut. Seule la
mémoire vidéo, limite dure, est comptée carte par carte (8, 12, 16 Go… ne donnent pas les mêmes
modèles dans une même classe).

| Classe | Bande passante utile | Calculée à | Cartes |
| --- | --- | --- | --- |
| S | 1000 Go/s et plus | 1000 | RTX 4090, RTX 5090, RTX 3090 Ti |
| A | 750 à 1000 | 750 | RTX 5080, RX 7900 XTX, RTX 3090, RTX 3080 Ti, RTX 5070 Ti, RX 7900 XT, RTX 3080 |
| B | 560 à 750 | 560 | RTX 4080, RTX 4070 Ti Super, RTX 5070, RX 9070 XT, RX 7800 XT, RX 6950 XT, RX 7900 GRE, RTX 3070 Ti, RTX 2080 Ti |
| C | 420 à 560 | 420 | RX 6800 XT, RX 6900 XT, RTX 4070 Super, RTX 4070, RTX 5060 Ti, RTX 3070, RTX 3060 Ti, RX 7700 XT, RX 6750 XT |
| D | 300 à 420 | 300 | RX 6700 XT, RTX 3060 12 Go, Arc A770, Arc B580, RX 9060 XT, RTX 2060, GTX 1660 Super |
| E | 220 à 300 | 220 | RTX 4060 Ti, RTX 4060, RX 7600, RX 6650 XT, RX 6600, RTX 3060 8 Go, RX 580 |
| F | moins de 220 | 150 | GTX 1650, RX 6500 XT, Arc A380, cartes portables d'entrée de gamme |

Processeur : 4 fils de calcul pour 8 à 11 cœurs (comme la référence), 3 pour 6 à 7, 6 pour 12 à 15,
8 pour 16 et plus.

### Les calculs

Avec s = 576 / bande passante de la classe de la carte :

- part de la carte prise par WoW : u = 1 sans plafond ; avec un plafond C,
  u = C / (images par seconde possibles), où les images possibles = 41,5 × (6,14 Mpx / pixels de
  l'image) / s / facteur de qualité (1 en qualité 10, 0,35 en qualité 1) ;
- latence sur la carte = latence de référence × s × (0,6 + 0,4 × u) ;
- capacité sur la carte = 20 × 5 % / (baisse de référence × s × a(u)), avec a(u) = 1 si u ≥ 0,9,
  0,35 si u ≤ 0,6, et entre les deux en ligne droite ;
- latence sur le processeur = latence de référence × √(4 / fils), fils de la classe du processeur ;
- capacité sur le processeur = 20 × 5 % / (baisse de référence × max(1, 8 / cœurs de la classe)).

| Valeurs de référence (mesurées le 09/11/2026) | Latence carte | Baisse carte (20 par min) | Latence processeur | Baisse processeur |
| --- | --- | --- | --- | --- |
| EuroLLM 1.7B | 0,69 s | 1,6 % | 0,95 s | 0,6 % |
| TranslateGemma 4B | 1,42 s | 4,1 % | 2,88 s | 0,7 % (rien de mesurable) |
| SalamandraTA 7B | 1,27 s | 6,1 % | trop lent | - |

### Mémoire

Un modèle n'est possible sur la carte que si
empreinte + WoW + autres programmes + 1 Go ≤ mémoire de la carte,
WoW étant compté pour le plus grand de sa mesure et de 7 Go (raids compris) ; **retail : 12 Go**
(version 2.4 : 11,7 Go mesurés au maximum des réglages ; 10 Go estimés en version 2.3). Sur le
processeur : AVX2, 4 cœurs pour le pack low, 8 pour les autres, et la mémoire vive libre.

**Pendant le jeu** (version 2.4) : la mémoire vidéo libre est relevée toutes les 5 s ; sous 1 Go
à deux relevés de suite, l'étape en cours est notée « mémoire » (pour cette version du jeu et cette
taille d'écran) et la chaîne se replie, comme pour un manque au chargement. Raison : le modèle est
souvent chargé avant que WoW n'atteigne sa mémoire de jeu (écran de connexion, entrée en jeu) ; le
09/11/2026, le 4B a ainsi été chargé sur la carte en retail avec 0,2 Go de marge réelle une fois le
jeu lancé, la situation qui avait fait planter le pilote avec le 12B.

### Version du jeu (2.3)

Chaque version du jeu a sa propre estimation : `ct-spike estimation --wow <dossier> --flavor
_retail_` lit le `Config.wtf` de cette version, ne prend que ses tests en jeu et compte WoW selon
elle. Les tests en jeu notent la version testée (« version=_retail_ », lue sur l'exécutable de WoW) ;
ceux d'avant la 0.11 ont tous été faits sur Forever (un échec noté en jeu porte aussi sa version
depuis la 0.12.6).

| | Forever | retail |
| --- | --- | --- |
| Mémoire vidéo comptée pour WoW | 7 Go | 12 Go (mesuré 11,7, version 2.4) |
| Coût d'une image à réglages égaux (jeu plafonné) | 1 | 1,5 (estimation) |

**Série retail du 09/11/2026** (0.12.5 ; retail 12.1.5, 3840x1600 plein écran, qualité maximale,
**échelle de rendu 200 %**, 41,9 images/s sans traduction, non plafonné ; WoW jusqu'à 11,7 Go,
2,9 Go de mémoire vidéo libres au plus bas sans modèle) :

| Étape | Latence médiane | Baisse d'images/s (20 messages/min) | Capacité sous 5 % | Verdict |
| --- | --- | --- | --- | --- |
| Pack low (EuroLLM 1.7B), carte | 0,39 s | 1,9 % | 53 / min | compatible, sûr ; 1,8 Go libres modèle chargé |
| Pack low, processeur | 0,92 s | 8,1 % | 12 / min | compatible, juste au seuil |
| TranslateGemma 4B, carte | - | - | - | impossible : 2,9 Go libres, 2,7 + 1 Go nécessaires |
| TranslateGemma 4B, processeur | 2,98 s | 15,0 % | 6 / min | trop lourd |
| SalamandraTA 7B, carte | - | - | - | impossible : 4,3 + 1 Go nécessaires |

Aucune réinitialisation du pilote, à-coups sous 55 ms. Sur le PC de référence en retail au
maximum, **seul le pack low tient** (français, anglais, espagnol, italien, allemand, russe) : la
chaîne est pack low sur la carte, puis sur le processeur. Les 23 langues (4B) demandent de libérer
de la mémoire vidéo (échelle de rendu à 100 %, à mesurer) ou une carte de plus de 16 Go. Sur
Forever, au maximum et à la même taille d'écran, le 4B tient sur la carte (1,42 s).

### Niveaux

| Niveau | Condition | Effet |
| --- | --- | --- |
| sûr | latence ≤ 2,4 s (80 % du seuil) et capacité ≥ 18 par minute | dans la chaîne ; arrête les téléchargements |
| probable | sous les seuils | dans la chaîne |
| limite | jusqu'à 25 % au-delà d'un seuil | proposé sur demande, avec avertissement |
| non | au-delà, mémoire insuffisante, ou interdit après un échec | pas proposé |

Seuils : latence médiane **3 s, sur la carte comme sur le processeur** (seuil unique depuis la
version 2.2 : la gêne pour le jeu est comptée à part) ; **12 messages par minute** (un toutes les
5 s, chat très animé) en gardant la baisse sous 5 % ; à-coups sous 100 ms ou le double de ce que
fait WoW seul ; aucune réinitialisation du pilote. Pour une même qualité, la chaîne prend d'abord
le plus rapide : la carte avant le processeur quand elle va plus vite.

### Un test sur ce PC remplace la prévision

Un test en jeu fait sur ce PC dans les mêmes conditions (même taille d'écran) remplace la latence et
la capacité prévues ; s'il a réussi, le niveau est au moins « probable ». Un « trop lourd » est
rejugé avec les seuils actuels (latence et baisse sont notées), sauf s'il venait d'à-coups ou d'un
jeu ralenti au repos : il reste un échec. Les essais d'avant la
0.8.7, sans conditions notées, ne valent pas mesure. Un **plantage** ou un **manque de mémoire**
interdit ce modèle et tous ceux qui occupent au moins autant de mémoire **au même endroit**,
quelles que soient les conditions.

## 4. Chaîne de repli et téléchargements

- **Chaîne** : les modèles « sûrs » et « probables », du meilleur en qualité au moins bon, et pour
  une même qualité du plus rapide au plus lent (carte ou processeur). Le pack low termine toujours
  la chaîne.
- **Téléchargés d'office** : les modèles de la chaîne, du premier jusqu'au premier « sûr » inclus,
  plus le pack low. Un modèle « sûr » a peu de chances d'échouer : ce qui est en dessous ne sert
  qu'en cas de malchance et reste sur demande.
- **Modèle actif** : le premier de la chaîne qui est installé.

Ce que la règle donne, par classe (estimation, 6 à 8 cœurs ; « 4B processeur » seulement à partir
de 8 cœurs) :

| Classe et mémoire | WoW qualité max., sans plafond | WoW 1440p qualité max., 60 images/s | WoW 1080p qualité 7, 60 images/s |
| --- | --- | --- | --- |
| S, 24 Go et plus | **7B** sûr → 4B → low ; 7B + low (5,5 Go) | pareil | pareil |
| A, 16 à 24 Go | **7B** sûr → 4B → low ; 7B + low | pareil | pareil |
| B, 16 Go (ta carte) | **7B** → 4B sûr → 4B processeur → low ; les trois (7,8 Go) | pareil | **7B** sûr ; 7B + low |
| C, 16 Go | **4B** (1,95 s) → 4B processeur → low ; 4B + low (3,3 Go) ; 7B limite | pareil | **7B** sûr ; 7B + low |
| B ou C, 12 Go | **4B** → 4B processeur → low ; 4B + low (le 7B ne tient pas à côté de WoW) | pareil | 4B sûr |
| D, 12 Go | **4B** (2,73 s) → 4B processeur → low ; 4B + low | pareil | 4B (2,44 s) |
| E, toutes mémoires | pack low ; 4B limite (3,7 s) ; avec 8 cœurs, **4B sur le processeur** d'abord | pareil | pareil |
| F, ou 8 Go et moins, ou 10 Go | pack low sur le processeur ; avec 8 cœurs, **4B sur le processeur** d'abord | pareil | pareil |

Avec 8 Go ou 10 Go, rien ne tient sur la carte à côté des 7 Go comptés pour WoW, même dans les
classes hautes (RTX 3070, RTX 3080 10 Go) : la traduction passe sur le processeur.

Les prévisions des cartes autres que la référence sont prudentes : la latence y est extrapolée de
celle mesurée sous forte charge sur la référence. L'usage réel les corrige (section 5).

## 5. En jeu : repli sans test

L'usage normal sert de test permanent : la traduction mesure sa latence, l'addon envoie les images
par seconde et la plus longue image, et l'application lit les journaux de Windows.

1. **Espacement** : au plus une traduction toutes les 60 / capacité secondes (7B sur la
   référence : une toutes les 4,4 s). Les messages en plus attendent leur tour : la baisse reste
   sous 5 % quel que soit le chat, c'est la latence qui s'allonge quand le chat déborde.
2. **Régulateur** : un à-coup pendant une traduction fait passer la traduction un cran plus
   prudent (priorité minimale, rafales plus courtes, pauses plus longues), sans changer de modèle.
3. **Échec** : étape suivante de la chaîne, notée dans `essais.tsv` (la chaîne du lancement
   suivant en tient compte) :
   - **plantage du pilote ou du serveur, manque de mémoire, à la carte** : ce modèle et les plus
     lourds n'y reviennent plus. Le même modèle **sur le processeur** reste permis : c'est la
     règle « plante sur la carte mais pas sur le processeur ». Il est l'étape suivante s'il est
     dans la chaîne ; sinon, le modèle plus léger sur la carte ; si le pack low plante sur la
     carte, tout passe sur le processeur ;
   - **trop lourd pour le jeu** (baisse ou à-coups au-delà des seuils malgré le régulateur) :
     pareil, les modèles plus légers sur la carte restent permis ;
   - **trop lent** : le prochain modèle de la chaîne prévu plus rapide.
4. **Changement de conditions** (écran, qualité, plafond) : les mesures d'un autre écran ne
   valent plus, la prévision reprend la main ; les interdictions après plantage restent.

## 6. Optimisations

Faites en 0.8.8, vérifiées dans les journaux de la série du 09/11/2026 :

1. **Cache complet de la fenêtre glissante pour TranslateGemma** (`--swa-full`) : le serveur
   recalculait 21 jetons à chaque reprise de rafale ; il n'en relit plus qu'un. 4B : 1,71 s en
   0.8.6, 1,57 s en 0.8.7, 1,42 s en 0.8.8.
2. **4 emplacements de conversation** (`-np 4 -c 2048`) : chacun garde la consigne de sa paire
   de langues. En usage réel (le joueur lit toujours dans sa langue), le message suivant de la même
   paire ne relit que son texte : TranslateGemma relit environ 20 jetons au lieu de 93 (−0,35 s sur
   la carte, environ −1 s sur le processeur). Le test alterne les langues : il n'en profite qu'un
   peu (consigne du 4B relue sur 75 à 82 jetons au lieu de 91 à 95), ses latences restent le pire
   cas.
3. **Capacité et espacement** au lieu d'une baisse fixe au rythme du test : le 7B devient
   utilisable sur le PC de référence (16 messages par minute) sans dépasser 5 %.
4. **Empreintes du PC de référence** pour les joueurs sans mesure : la RTX 3060 12 Go et les cartes
   de 12 Go ne perdent plus le 4B pour 0,7 Go comptés en trop.
5. Série : le 4B sur le processeur est mesuré ; une mesure incomplète (WoW à l'arrière-plan) n'est
   plus un échec.

À faire avec la traduction en jeu (phase 1) :

6. **Cache des traductions** : les annonces répétées du canal commerce ne sont traduites qu'une
   fois (coût nul ensuite).
7. Ne traduire que ce qui doit l'être : messages déjà dans la langue du joueur ignorés, canaux
   choisis par le joueur.
8. À mesurer : paquets de 20 à 30 opérations pendant la génération (un jeton à la fois, donc pas de
   long calcul) pour raccourcir la latence du 4B sous charge.
9. Écarté pour l'instant : décodage spéculatif (SalamandraTA 2B devant le 7B) : environ 25 % de
   charge en moins pour 1,5 Go de mémoire vidéo en plus.

## 7. Reste à mesurer

- **Retail à l'échelle de rendu 100 %** (version 2.4) : la série du 09/11/2026 (200 %) ne laisse
  que le pack low ; à 100 %, le 4B tient-il sur la carte avec 1 Go de marge ? Le coût d'une image
  retail (1,5) reste une estimation.
- Une seconde configuration (carte NVIDIA, ou 8 à 12 Go) : `ct-spike estimation` puis la série,
  pour comparer la prévision à la mesure ailleurs que sur le PC de référence (table des cartes,
  extrapolation de la latence, règle de mémoire).
- Le 4B sur le processeur d'un autre 8 cœurs (portable surtout) : 2,88 s sur le 5800X3D, tout
  près du seuil de 3 s.
- PC à deux cartes (intégrée + dédiée) : faire traduire la carte que WoW n'utilise pas éviterait
  tout partage ; à vérifier.
