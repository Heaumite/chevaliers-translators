# Code signing policy / Politique de signature du code

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

*Signature gratuite du code fournie par SignPath.io, certificat de SignPath Foundation. Version
française plus bas.*

## Project

Chevaliers Translators is an open source (GPL-3.0-or-later) World of Warcraft addon with a
Windows companion application. The addon relays the in-game chat through a pixel banner on the
screen; the application reads it, translates the messages on the player's PC (llama.cpp, no online
service) and draws the translated chat over the game. Source code:
<https://github.com/Heaumite/chevaliers-translators>. Downloads:
<https://github.com/Heaumite/chevaliers-translators/releases>.

## Signed files

Only executables built from this repository by GitHub Actions
(`.github/workflows/compilation.yml`) are signed: `ChevaliersTranslators.exe` (the player's
application) and `ct-outils.exe` (developer tools), product name "Chevaliers Translators", version
of `app/Cargo.toml`. Programs and files that the application downloads at run time (the llama.cpp
translation server, translation models) are third-party components, are not part of the signed
files and keep their own licenses.

## Team roles

- Committers and reviewers: [Heaumite](https://github.com/Heaumite) (author and maintainer)
- Approvers: [Heaumite](https://github.com/Heaumite)

Every signing request is approved manually. Team members use multi-factor authentication on
GitHub and SignPath.

## Privacy policy

This program will not transfer any information to other networked systems unless specifically
requested by the user or the person installing or operating it. In detail:

- It downloads the llama.cpp translation engine from GitHub (`api.github.com`, `github.com`) and
  translation models from Hugging Face (`huggingface.co`), on first launch and when the user asks
  for a model. These requests contain only the names of the requested files.
- Chat messages are translated on the player's PC, by a local server listening on `127.0.0.1`. No
  message, player name or game data is sent anywhere.
- The application reads, locally, a small area at the bottom right of the game window (the addon's
  pixel banner) and the position of the game window. It does not record, keep or send
  screenshots.
- It uses the clipboard and simulates Ctrl+V then Enter only after the player presses Shift+Enter
  in the chat box, to send the player's own translated text; the previous clipboard content is put
  back afterwards.
- No telemetry, no analytics, no account.

Files written by the program: `%LOCALAPPDATA%\ChevaliersTranslators` (models, engine, logs, in-game
measurements) and the addon folders `Interface\AddOns\ChevaliersTranslators` and
`Interface\AddOns\ChevaliersTranslators_Live` of each World of Warcraft version.

## Uninstallation

Tray icon menu, **Désinstaller...**: the addon is removed from every World of Warcraft version
found and, if the user agrees, the models, engine, logs and addon settings too. By hand: delete
the two addon folders above in each game version, then `%LOCALAPPDATA%\ChevaliersTranslators`,
then the executable.

---

# Politique de signature du code (français)

Signature gratuite du code fournie par [SignPath.io](https://about.signpath.io/), certificat de
[SignPath Foundation](https://signpath.org/).

## Projet

Chevaliers Translators est un addon de World of Warcraft au code ouvert (GPL-3.0 ou ultérieure),
avec une application compagnon pour Windows. L'addon relaie le chat du jeu par un bandeau de pixels
à l'écran ; l'application le lit, traduit les messages sur le PC du joueur (llama.cpp, aucun
service en ligne) et redessine le chat traduit par-dessus le jeu.

## Fichiers signés

Seuls les exécutables compilés depuis ce dépôt par GitHub Actions sont signés :
`ChevaliersTranslators.exe` et `ct-outils.exe`, produit « Chevaliers Translators », version de
`app/Cargo.toml`. Le serveur llama.cpp et les modèles, téléchargés par l'application, sont des
composants tiers non signés par ce projet, sous leurs propres licences.

## Rôles

- Auteur, relecteur : [Heaumite](https://github.com/Heaumite)
- Approbateur : [Heaumite](https://github.com/Heaumite)

Chaque demande de signature est approuvée à la main ; double authentification sur GitHub et
SignPath.

## Confidentialité

Le programme n'envoie aucune information sur le réseau sans demande de l'utilisateur :

- il télécharge le moteur llama.cpp sur GitHub et les modèles de traduction sur Hugging Face (au
  premier lancement et quand le joueur demande un modèle) ; ces requêtes ne contiennent que le nom
  des fichiers demandés ;
- les messages sont traduits sur le PC du joueur (serveur local sur `127.0.0.1`) : aucun message,
  nom de joueur ou donnée du jeu n'est envoyé ;
- l'application lit, sur place, une petite zone en bas à droite de la fenêtre du jeu (le bandeau de
  l'addon) et la position de cette fenêtre ; aucune capture d'écran n'est gardée ni envoyée ;
- le presse-papiers et Ctrl+V puis Entrée simulés ne servent qu'après Maj+Entrée du joueur dans la
  boîte de saisie, pour envoyer son propre texte traduit ; le presse-papiers d'avant est remis ;
- ni télémétrie, ni statistiques, ni compte.

## Désinstallation

Menu de l'icône près de l'horloge, **Désinstaller...** : l'addon est retiré de toutes les versions
de WoW et, au choix, les modèles, le moteur, les journaux et les réglages de l'addon. À la main :
supprimer les dossiers `ChevaliersTranslators` et `ChevaliersTranslators_Live` de
`Interface\AddOns` dans chaque version du jeu, puis `%LOCALAPPDATA%\ChevaliersTranslators`, puis
l'exécutable.
