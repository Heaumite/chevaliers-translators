# Paquet de distribution : l'addon et l'application dans un seul zip, dossier ChevaliersTranslators
# (à décompresser dans Interface\AddOns, comme tout addon). Lancée depuis ce dossier,
# l'application pose ensuite l'addon dans les autres versions du jeu.
#
#   powershell -ExecutionPolicy Bypass -File outils\paquet.ps1 [-Sortie <dossier>] [-Exe <exécutable>]
#
# -Exe : exécutable déjà compilé (et signé) à mettre dans le paquet, sans recompiler (GitHub
# Actions, après la signature par SignPath). Le zip contient aussi la licence (GPL-3.0) ; une
# empreinte SHA-256 du zip est écrite à côté.
param([string]$Sortie = (Join-Path $PSScriptRoot "..\dist"), [string]$Exe = "")
$ErrorActionPreference = "Stop"

$racine = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$app = Join-Path $racine "app"
$version = (Select-String -Path (Join-Path $app "Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$toc = Get-Content (Join-Path $racine "addon\ChevaliersTranslators\ChevaliersTranslators.toc")
if (-not ($toc -contains "## Version: $version")) { throw ".toc de l'addon pas en version $version" }

if ($Exe -eq "") {
    Push-Location $app
    try {
        cargo build --release --target-dir target-win --bin ChevaliersTranslators
        if ($LASTEXITCODE -ne 0) { throw "compilation de l'application impossible" }
    } finally { Pop-Location }
    $Exe = Join-Path $app "target-win\release\ChevaliersTranslators.exe"
}

$source = Join-Path $racine "addon\ChevaliersTranslators"
# Chemin dans le zip : sous-dossiers de l'addon gardés (Media\Logo.tga), « / » comme séparateur.
$entrees = @(Get-ChildItem $source -File -Recurse | ForEach-Object {
    @{ Fichier = $_; Nom = "ChevaliersTranslators/" + $_.FullName.Substring($source.Length + 1).Replace("\", "/") }
}) + @(
    @{ Fichier = (Get-Item $Exe); Nom = "ChevaliersTranslators/ChevaliersTranslators.exe" },
    @{ Fichier = (Get-Item (Join-Path $racine "LICENSE")); Nom = "ChevaliersTranslators/LICENSE.txt" }
)
New-Item -ItemType Directory -Force $Sortie | Out-Null
$zip = Join-Path (Resolve-Path $Sortie).Path "ChevaliersTranslators-$version.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }

# Zip écrit par .NET avec des « / » : Compress-Archive de Windows PowerShell 5.1 met des « \ »,
# mal lus par certains gestionnaires d'addons.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$z = [System.IO.Compression.ZipFile]::Open($zip, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($e in $entrees) {
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z, $e.Fichier.FullName, $e.Nom)
    }
} finally { $z.Dispose() }

$empreinte = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Set-Content -Path "$zip.sha256" -Value "$empreinte  ChevaliersTranslators-$version.zip" -Encoding ascii
Write-Host "Paquet $version : $zip"
Write-Host "SHA-256 : $empreinte"
$entrees | ForEach-Object { Write-Host ("  " + $_.Nom + "  " + [math]::Round($_.Fichier.Length / 1KB) + " Ko") }
