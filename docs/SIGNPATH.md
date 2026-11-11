# Signature gratuite par SignPath Foundation : démarches

But : `ChevaliersTranslators.exe` signé par un certificat reconnu par Windows, gratuitement. Le
certificat appartient à SignPath Foundation : Windows affichera « Éditeur : SignPath Foundation ».
Conditions (https://signpath.org/terms) : code ouvert sous licence OSI (ici GPL-3.0), sans
composant propriétaire, projet maintenu et déjà publié, exécutables compilés depuis le dépôt public
par GitHub Actions, page « Code signing policy », double authentification, chaque signature
approuvée à la main.

Ce qui est déjà prêt dans le dépôt :

| Fichier | Rôle |
| --- | --- |
| `LICENSE` | GPL-3.0 (texte officiel) ; `GPL-3.0-or-later` dans `app/Cargo.toml` et le `.toc` |
| `.github/workflows/compilation.yml` | tests Linux et Windows, compilation, signature SignPath (sur étiquette `vX.Y.Z`), paquet, publication dans Releases |
| `signpath/artifact-configuration.xml` | ce que SignPath signe dans l'envoi : les deux `.exe`, nom du produit et version imposés |
| `docs/CODE_SIGNING_POLICY.md` | page demandée par SignPath : mention SignPath, rôles, confidentialité, désinstallation |
| `.gitignore`, `.gitattributes` | compilations hors du dépôt ; fins de ligne LF (les tests comparent le texte de l'addon) |
| menu **Désinstaller...** | désinstallation demandée par SignPath pour un programme qui installe quelque chose |

## La limite de 25 Mo de GitHub

25 Mo, c'est la limite d'un fichier **déposé par le navigateur** (bouton « Upload files »). Avec
Git (ligne de commande ou GitHub Desktop), un fichier peut faire jusqu'à 100 Mo, et un fichier joint
à une version (Releases) jusqu'à 2 Go. Rien ne s'en approche ici :

- le dépôt fait environ 4 Mo, le plus gros fichier 1 Mo (`media/logo.tga`, `media/banniere.tga`) ;
- l'exécutable n'est jamais déposé : GitHub Actions le compile et le joint à la version ;
- les modèles (plusieurs Go) ne vont jamais sur GitHub : l'application les télécharge sur Hugging
  Face chez le joueur ;
- `app/target-win` (compilations, plusieurs Go) est exclu par `.gitignore`.

## 1. Compte GitHub

1. Compte `Heaumite` : activer la double authentification (Settings > Password and authentication
   > Two-factor authentication). SignPath l'exige.
2. Créer le dépôt : bouton **New repository**, nom `chevaliers-translators`, **Public**, sans
   README, sans .gitignore, sans licence (le dépôt les apporte).

## 2. Mettre le code en ligne (PowerShell)

Git pour Windows, une fois : `winget install --id Git.Git -e`, puis rouvrir PowerShell.

```powershell
$ct = "C:\Dev\ChevaliersTranslators"
cd "$ct\chevaliers-translators"
git init -b main
git config user.name "Heaumite"
git config user.email "Heaumite@users.noreply.github.com"   # adresse anonyme de GitHub
git add .
git status        # vérifier : ni target-win, ni dist, ni .exe
git commit -m "Chevaliers Translators 0.14.3"
git remote add origin https://github.com/Heaumite/chevaliers-translators.git
git push -u origin main
```

Au premier `git push`, une fenêtre de connexion à GitHub s'ouvre (navigateur) : accepter.
Onglet **Actions** du dépôt : la compilation doit passer au vert (tests Linux, puis Windows).

## 3. Première version publique (non signée)

SignPath ne signe qu'un projet déjà publié : une première version sans signature.

```powershell
git tag v0.14.3
git push origin v0.14.3
```

GitHub Actions publie la version dans **Releases** : `ChevaliersTranslators-0.14.3.zip` (addon,
application, licence) et son empreinte `.sha256`.

## 4. Demande à SignPath Foundation

Formulaire : https://signpath.org/apply. Textes prêts (en anglais) :

- **Project name** : Chevaliers Translators
- **Repository** : https://github.com/Heaumite/chevaliers-translators
- **Homepage / download page** : https://github.com/Heaumite/chevaliers-translators/releases
- **License** : GPL-3.0-or-later
- **Code signing policy** : https://github.com/Heaumite/chevaliers-translators/blob/main/docs/CODE_SIGNING_POLICY.md
- **Build system** : GitHub Actions (`.github/workflows/compilation.yml`), Windows executables built
  with Rust (cargo) from the repository, signed artifact = GitHub artifact uploaded by
  `actions/upload-artifact`.
- **Description** :
  > Chevaliers Translators translates the World of Warcraft chat between European languages,
  > locally on the player's PC. A Lua addon relays chat messages through a small pixel banner
  > drawn on the screen; a Windows companion application (Rust) reads that banner, translates the
  > messages with a local llama.cpp server and open translation models (EuroLLM, TranslateGemma,
  > SalamandraTA, downloaded from Hugging Face on first launch), and draws the translated chat
  > over the game. Nothing is sent to an online service. The application is distributed inside
  > the addon's zip; unsigned, it triggers SmartScreen and antivirus warnings, which is why we ask
  > for code signing.
- **Team** : single maintainer, Heaumite (author, reviewer and approver), MFA enabled on GitHub
  and SignPath.
- **Signed files** : `ChevaliersTranslators.exe`, `ct-outils.exe` (product name "Chevaliers
  Translators", version from `app/Cargo.toml`).

SignPath Foundation n'est pas tenue d'accepter ; la réponse peut prendre quelques jours ou
semaines. Les versions continuent d'être publiées non signées en attendant.

## 5. Après l'acceptation : réglages

Dans SignPath (le projet est souvent créé par SignPath Foundation ; garder ces noms, le workflow
les utilise) :

1. Installer l'application GitHub de SignPath sur le dépôt `Heaumite/chevaliers-translators`.
2. Projet `chevaliers-translators`, relié au **Trusted Build System** « GitHub.com » et au dépôt.
3. Configuration d'artefact `executables` : coller le contenu de
   `signpath/artifact-configuration.xml`.
4. Règle de signature `release-signing` : certificat de SignPath Foundation, approbateur
   Heaumite (approbation manuelle de chaque version).
5. Jeton d'API d'un utilisateur « submitter » du projet.

Dans GitHub, dépôt > Settings > Secrets and variables > Actions :

| Onglet | Nom | Valeur |
| --- | --- | --- |
| Secrets | `SIGNPATH_API_TOKEN` | le jeton d'API de SignPath |
| Variables | `SIGNPATH_ORGANIZATION_ID` | l'identifiant d'organisation SignPath (page de l'organisation) |

Tant que la variable `SIGNPATH_ORGANIZATION_ID` n'existe pas, le workflow saute la signature.

## 6. Chaque nouvelle version, ensuite

1. Version mise à jour comme d'habitude (`app/Cargo.toml`, `.toc`, `CT.VERSION` de `Core.lua` ;
   un test le vérifie), puis `git commit` et `git push`.
2. `git tag vX.Y.Z` puis `git push origin vX.Y.Z` (même numéro que `app/Cargo.toml`, sinon le
   workflow s'arrête).
3. SignPath envoie une demande d'approbation (courriel, site) : l'approuver. Le workflow attend
   jusqu'à 2 h, récupère les exécutables signés, fait le paquet et publie la version.
4. Vérification sur n'importe quel PC :

```powershell
Get-AuthenticodeSignature .\ChevaliersTranslators.exe | Format-List Status, SignerCertificate
```

Attendu : `Status : Valid`, signataire `SignPath Foundation`. SmartScreen peut encore prévenir sur
les premières versions : la réputation se construit avec les téléchargements, version après version
signées par le même certificat.

Si SignPath refuse la version à signer (« product version mismatch ») : les exécutables portent la
version en quatre nombres (`0.14.3.0`, texte et nombres identiques, `app/build.rs`) et le workflow
passe la même valeur (`pe`) ; vérifier que l'étiquette correspond à `app/Cargo.toml`.
