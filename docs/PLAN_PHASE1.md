# Chevaliers Translators - plan de la phase 1 (le produit)

10/11/2026. Point de départ : le bilan de la phase 0 (go, trois réserves) et l'application 0.12.6.

## But

Une version que n'importe quel joueur installe et utilise sans console ni commande : l'addon
« Chevaliers Translators » dans toutes ses versions de WoW, et l'application compagnon qui
traduit sur son PC. Même fonctionnement que la phase 0 (bandeau de pixels, chat miroir, Maj+Entrée,
traduction locale, chaîne de repli), plus l'installation, les réglages, la robustesse et la
publication.

## Décisions prises (10/11/2026)

| Décision | Choix |
| --- | --- |
| Code source (11/11) | Ouvert, GPL-3.0 ou ultérieure, dépôt `Heaumite/chevaliers-translators` ; signature gratuite par SignPath Foundation |
| Premier jalon | J1 Fondation |
| Distribution | L'application est livrée **dans le zip de l'addon** (dossier `ChevaliersTranslators`, à côté des fichiers Lua). Lancée depuis là, elle retrouve son dossier de WoW et pose l'addon dans les autres versions |
| Commande en jeu | `/chevaliers` |

Risque de la distribution : CurseForge refuse les archives qui contiennent des « fichiers,
extensions ou dossiers sur liste noire », liste non publiée ; les exécutables n'y sont ni
autorisés ni interdits noir sur blanc. À vérifier tôt par un premier envoi (fichier en bêta)
avant de construire la suite dessus. Repli : l'addon seul sur CurseForge, l'application sur
GitHub Releases.

## Ce qui est repris de la phase 0

| Partie | Repris | Laissé aux outils de développement |
| --- | --- | --- |
| Addon | bandeau, relais du chat, miroir (lignes des joueurs), Maj+Entrée par collage, bouton Trad, présence de l'application, icônes | tests S1, calage, échantillon, perf, essais, diag (gardés, listés par `/chevaliers dev`) ; emplacements S6 retirés |
| Application | décodage, capture, miroir et fenêtres, détection des langues, dictionnaire, moteur et chaîne de repli, règle des modèles (2.4), matériel, polices et archives du jeu, interface des modèles | `ct-outils` : banc de tests (`bench`, `tests-en-jeu`), auto-test, diagnostic des plantages, installation à la main |

## Jalons

**J1 - Fondation** - livré en 0.13.0, corrigé en 0.13.1 (10/11/2026)
- 0.13.0 essayé dans retail le 10/11 : traduction, miroir et arrêt à la fermeture du jeu
  fonctionnent, mais « /reload » demandé alors que l'application tournait avant la connexion
  (présence jugée à la fin de l'écran de chargement). 0.13.1 : présence jugée à la lecture du
  fichier, écrite dès le lancement de l'application, remise à zéro quand on la quitte ;
  `/reload` demandé seulement si l'application est lancée après la connexion. Logo et bannière
  fournis : icône de l'exécutable et près de l'horloge, liste des addons, fenêtre des modèles.
  0.13.2 : style visuel de l'application tiré de la bannière et du logo (fenêtre des modèles,
  panneau affiché sur le jeu, barre de titre sombre).
- Dépôt réorganisé : `addon/ChevaliersTranslators/`, `app/` (bibliothèque `ct` + deux
  exécutables), `outils/` (harnais Lua, paquet de distribution), `docs/`.
- Addon renommé `ChevaliersTranslators` (dossier, `.toc`, réglages repris de `CT_SpikeDB`),
  commande `/chevaliers` (`on`, `off`, `langue`, `info`, `diag` ; outils de test sous
  `/chevaliers dev`) ; l'ancien `CT_Spike`, sa présence et ses emplacements désinstallés.
- `ChevaliersTranslators.exe` sans console : icône près de l'horloge (état, modèles de traduction,
  journal, Quitter), journal dans `%LOCALAPPDATA%\ChevaliersTranslators\journal` (10 derniers
  gardés), une seule instance, addon posé ou mis à jour au démarrage, traduction lancée dès que WoW
  est ouvert pour sa version et arrêtée 20 s après sa fermeture (mémoire vidéo rendue), serveur de
  traduction arrêté avec l'application même fermée brutalement.
- Versions de l'addon et de l'application vérifiées : l'application met l'addon à jour elle-même
  et le dit par une bulle (« /reload » ou « redémarre le jeu ») ; l'addon signale une application
  d'une autre version.
- Pause : `/chevaliers off` dans le jeu (gardé aux connexions suivantes). Pas de pause dans le menu
  de l'icône : l'application ne peut pas parler à l'addon pendant la partie, le chat officiel ne
  reviendrait qu'au `/reload`.

**Retours du 10/11 (0.13.3)** : erreurs Lua en instance (valeurs secrètes) corrigées, messages
du combat relayés et traduits après ; chat traduit sur l'écran de WoW (télévision) ; tailles de
police égales ; bandeau immobile et sombre ; portée des canaux pour Trad:AUTO ; fautes de frappe.

**0.14.0 (livré le 11/11)** : tous les modèles qui peuvent tourner téléchargés au premier
lancement (débit bridé WoW ouvert), le meilleur actif ; mesures discrètes pendant le jeu
(`mesures.tsv`) ; proposition d'un modèle supérieur hors combat avec avertissement
(`/chevaliers monter`, `/chevaliers non`). Retours du 11/11 : pas de mode fenêtré imposé pour les
télévisions, mais un **dispositif télévision** appliqué quand l'écran de WoW est reconnu comme
une télévision avant la connexion (texte du jeu laissé dessous, chat traduit qui le recouvre sur
une plaque sombre ; `/chevaliers tele auto|on|off`) ; en combat rien n'est redessiné ni affiché,
le chat traduit revient hors combat avec ce qui a été traduit.
**0.14.1 (11/11)** : essai de la 0.14.0 sur le PC de référence : traduction jamais démarrée
(blocage entre le fil de capture et le fil principal, présent depuis la 0.13.3 : présence plus
écrite, rien de redessiné). Corrigé.
**0.14.2 (11/11)** : plus de réserve de 1 Go de mémoire vidéo : gouverneur en jeu (descente près
de la saturation, même en combat ; remontée hors combat quand la place revient ou quand les
réglages graphiques de WoW changent) ; un manque de mémoire n'interdit plus un modèle. Éditeur
« Chevaliers » dans les informations de version des exécutables, signature propre au PC
(`outils/signer.ps1`) ; un certificat reconnu reste à prendre pour la publication (J6).
**0.14.3 (11/11)** : code ouvert (GPL-3.0 ou ultérieure), préparation de la signature gratuite
par SignPath Foundation : GitHub Actions (tests, compilation, signature, publication), politique
de signature, désinstallation depuis le menu de l'icône ; démarches dans `docs/SIGNPATH.md`.
Retours du 11/11 midi : erreur Lua en champ de bataille (absence secrète) corrigée ; premier
modèle choisi avec WoW compté 7 Go à la première installation, puis d'après les mesures réelles.
**0.15** : mises à jour des modèles suivies sur Hugging Face, vérifiées WoW fermé puis essayées
pendant une absence (AFK) avant d'être proposées.

**J2 - Installation et démarrage**
- Premier lancement guidé : WoW trouvé, addon posé dans toutes les versions, moteur llama.cpp et
  pack low téléchargés, matériel détecté, modèles proposés (interface de la phase 0).
- Démarrage avec Windows (au choix) ; traduction active à la connexion comme aujourd'hui.
- Mises à jour : l'application vérifie sa version et remet l'addon à jour dans toutes les versions.
- Icône de l'exécutable (ressource Windows) et paquet de distribution vérifié sur CurseForge.

**J3 - Réglages**
- Panneau d'options dans le jeu : langue d'envoi par défaut, traduire mes messages, canaux à ne
  pas traduire, affichage de l'acronyme, original au survol.
- Interface de l'application : état, modèle actif, statistiques, dictionnaire personnel (termes
  ajoutés par le joueur, sans recompiler), dossier des modèles.

**J4 - Robustesse** (réserves 1 et 3 du bilan)
- Repli quand les images par seconde baissent pendant l'usage normal.
- Addons de chat (ElvUI, Prat, Chattynator) : miroir adapté, ou à défaut traduction affichée sous
  le message.
- Tests restants : QuestVoice complet (S7), retail à l'échelle de rendu 100 %, un second PC
  (NVIDIA ou 8 à 12 Go), une soirée de raid et une capitale avec les compteurs du bandeau.

**J5 - Qualité de traduction** (réserve 2)
- Corpus de vrai chat (anonymisé), note de qualité par modèle et par paire de langues.
- Petit modèle meilleur que le pack low, ou réglages plus stricts ; nettoyage des réponses
  (« French: ») ; dictionnaire étendu.

**J6 - Publication**
- Licences : EuroLLM et SalamandraTA (Apache 2.0), TranslateGemma (conditions Gemma), llama.cpp
  (MIT) : page d'attributions, conditions acceptées au téléchargement.
- Règles de Blizzard : lecture de l'écran (comme QuestVoice) et Ctrl+V + Entrée simulés après ton
  Maj+Entrée, à vérifier ; repli possible : coller seulement, Entrée par le joueur.
- CurseForge : zip de l'addon avec l'application (décision du 10/11) ; pas de lien de
  téléchargement externe sur la page.
- Signature de l'application par un certificat reconnu, éditeur « Chevaliers » (Azure Trusted
  Signing ou SignPath pour un projet au code ouvert ; SmartScreen, antivirus : fenêtres par-dessus le jeu, touches
  simulées, lecture de l'écran) ; bêta fermée avant la sortie.

## Décisions encore ouvertes

| Décision | Recommandation |
| --- | --- |
| Langue de l'interface | Français d'abord, anglais ensuite |
| Ordre des jalons | J2, J3, puis J4 et J5 en parallèle, J6 après une bêta |
