# Chevaliers Translators - bilan de la phase 0

10/11/2026 - application 0.12.6. Établi d'après les critères posés avant les derniers tests
(`README.md`, « Clôture de la phase 0 »), sur le PC de référence : Ryzen 7 5800X3D, 64 Go,
Radeon RX 6950 XT 16 Go, écran 3840x1600, client français, WoW Forever 1.60.1 et retail 12.1.5.

## Verdict : GO pour la phase 1, avec trois réserves

Le principe tient de bout en bout : un addon qui parle à une application par un bandeau de
pixels, une application qui traduit sur la machine du joueur sans service en ligne, et un chat
redessiné par-dessus celui du jeu, avec envoi traduit en un appui. Aucune des réserves ne remet ce
principe en cause ; toutes se traitent dans le produit.

## Ce que la phase 0 devait prouver

| Test | Question éliminatoire | Résultat |
| --- | --- | --- |
| S1 | Le bandeau passe de WoW à Windows sans erreur, assez vite ? | Oui : 1000/1000 puis 2000/2000 trames, 0 illisible, 20 trames/s (5,1 à 5,7 Ko/s). Sous le spam des canaux retail, deux défauts corrigés en 0.12.3 (miroir caché quand la file ne se vidait pas, trames manquées pendant le dessin) |
| S2 | Le chat reçu sort du jeu ? | Oui : tous les messages de joueurs, sur tous les canaux ; les messages « secrets » de retail ignorés |
| S3 | Un message traduit part du jeu ? | Oui : Maj+Entrée, traduction collée puis envoyée, un seul appui, tous les canaux, liens d'objets gardés |
| S5a | L'application retrouve la police exacte du chat ? | Oui : archives du jeu lues en 0,7 s, police vérifiée |
| S5 | Le chat peut être redessiné à l'identique ? | Oui : calage au pixel (199 pixels non recouverts sur tout le texte), même coupe des lignes, même effacement ; menus et bulles du jeu au-dessus ; depuis 0.12.5, seules les lignes des joueurs sont redessinées |
| S4a | Le matériel de chaque joueur est reconnu ? | Oui : carte, processeur, mémoire, moteurs de calcul |
| S4b | Un modèle traduit pendant le jeu sans le gêner ? | Oui sur Forever (4B sur la carte, 1,42 s, baisse 4,1 %) ; sur retail au maximum, le pack low seulement (voir réserve 1). Un plantage du pilote (12B, 09/11) a donné la règle du 1 Go libre |
| S4c | Les modèles s'installent et se replient chez le joueur ? | Oui : pack low d'office, autres sur demande, un seul actif, chaîne de repli en jeu |
| S8 | La traduction réelle fonctionne dans le chat ? | Oui, validé le 10/11 en 0.12.5 : messages étrangers traduits à leur place (« [EN] bonjour »), les tiens aussi, active dès la connexion |
| S9 | Retail est couvert ? | Oui pour le chat ; série de performances faite (réserve 1) |
| S7 | Coexistence avec QuestVoice ? | Partiel : 1000/1000 trames avec QuestVoice chargé (08/11) ; lecture des quêtes pendant la traduction pas encore testée (reporté, réserve 3) |

## Critères du go

| Critère | Seuil | Mesuré | |
| --- | --- | --- | --- |
| S8 | les quatre vérifications | validées le 10/11 | atteint |
| S9 : un modèle au moins « sûr » ou « probable » dans retail au maximum | latence ≤ 3 s, ≥ 12 messages/min sous 5 % de baisse, à-coups < 100 ms, aucune réinitialisation | pack low sur la carte : 0,39 s, 53 messages/min, baisse 1,9 %, à-coups < 55 ms, aucune réinitialisation | atteint (sûr) |
| S1 avec QuestVoice | 2000/2000 | 1000/1000 (08/11) | partiel |
| S7 | QuestVoice lit toutes les quêtes, aucune trame manquée | non fait | reporté |

## Réserves

**1. Retail au maximum : seul le pack low tient sur une carte de 16 Go.** À l'échelle de rendu
200 %, WoW occupe jusqu'à 11,7 Go et ne laisse que 2,9 Go : le 4B (2,7 Go + 1 Go de marge) et le
7B n'y entrent pas, et le 4B sur le processeur coûte 15 % d'images par seconde. Le joueur garde
la traduction entre français, anglais, espagnol, italien, allemand et russe, pas les 23 langues.
La 0.12.6 compte désormais WoW retail pour 12 Go (règle 2.4) et surveille la mémoire vidéo pendant
le jeu : le 09/11, `live` avait chargé le 4B sur la carte avant que WoW n'atteigne sa mémoire de
jeu, avec 0,2 Go de marge réelle, la situation qui avait fait planter le pilote avec le 12B.
Phase 1 : mesurer retail à l'échelle de rendu 100 %, et dire au joueur dans l'interface ce que
coûte chaque réglage (« échelle de rendu à 100 % : 23 langues »).

**2. Qualité du pack low.** La série montre ses défauts sur le corpus de test : « tank » devenu
« réservoir », Mortemines traduites mot à mot (« mines de charbon »), préfixes parasites
(« French: », « Ruso: »), une recopie sans traduction. Le dictionnaire de la 0.12.4 règle les noms
de lieux et le jargon ; le reste est la qualité du modèle. Phase 1 : mesurer la qualité sur du
chat réel (pas seulement la vitesse), et chercher un meilleur petit modèle ou un réglage plus strict.

**3. Points non couverts par le kit.** Coexistence complète avec QuestVoice (S7) ; repli quand
les images par seconde baissent pendant l'usage normal (seuls plantage, mémoire et lenteur sont
suivis) ; capture par Windows Graphics Capture (S1b) ; une seconde configuration (carte NVIDIA, ou
8 à 12 Go) pour vérifier la prévision ailleurs que sur le PC de référence ; confirmation, en raid
ou en capitale, que les compteurs `[bandeau]` de la 0.12.3 restent à zéro sous le spam.

## Ce que la phase 0 laisse à la phase 1

- **Gardé tel quel** : protocole du bandeau, relais du chat, chat miroir (lignes des joueurs
  seulement), Maj+Entrée par collage, détection des langues, moteur de traduction et sa chaîne de
  repli, règle de choix des modèles (`REGLE_MODELES.md`, 2.4), dictionnaire des termes.
- **À retirer** : outils de la phase 0 (tests S1, emplacements S6, calage, échantillons,
  commandes de diagnostic visibles du joueur).
- **À écrire** : l'addon publié « Chevaliers Translators » et l'application pour tous
  (installation, démarrage avec WoW, interface des modèles, journal), le repli sur baisse
  d'images par seconde, la mesure de qualité, et les tests restants de la réserve 3.
