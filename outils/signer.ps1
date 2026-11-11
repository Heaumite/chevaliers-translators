# Signature des exécutables avec l'éditeur « Chevaliers » (0.14.2), sur ce PC.
#
# Crée une fois un certificat de signature de code « Chevaliers » propre à ce PC (clé non
# exportable, dans le magasin de l'utilisateur), le déclare digne de confiance sur ce PC seulement
# (Windows demande une confirmation la première fois), puis signe ChevaliersTranslators.exe et
# ct-outils.exe. Windows affiche alors « Éditeur : Chevaliers » (Propriétés > Signatures
# numériques, demandes d'autorisation).
#
# Limite : ce certificat ne vaut que sur ce PC. Pour les autres joueurs (SmartScreen, Smart App
# Control), il faut un certificat délivré par une autorité reconnue (Azure Trusted Signing,
# SignPath pour les projets au code ouvert, ou un certificat de signature de code payant).
#
# Usage, après « cargo build --release --target-dir target-win », depuis le dossier app :
#   powershell -ExecutionPolicy Bypass -File ..\outils\signer.ps1

param([string]$Dossier = (Join-Path $PSScriptRoot "..\app\target-win\release"))

$sujet = "CN=Chevaliers"
$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Where-Object { $_.Subject -eq $sujet -and $_.NotAfter -gt (Get-Date).AddDays(30) } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) {
    Write-Host "Création du certificat « Chevaliers » pour ce PC..."
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $sujet `
        -FriendlyName "Chevaliers (signature de Chevaliers Translators)" `
        -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy NonExportable `
        -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -NotAfter (Get-Date).AddYears(5)
    $cer = Join-Path $env:TEMP "chevaliers-signature.cer"
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    # Confiance sur ce PC, pour cet utilisateur : Windows demande de confirmer (répondre Oui).
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\CurrentUser\Root | Out-Null
    Remove-Item $cer -ErrorAction SilentlyContinue
}
Write-Host ("Certificat : {0} (valable jusqu'au {1:dd/MM/yyyy})" -f $cert.Subject, $cert.NotAfter)

foreach ($exe in "ChevaliersTranslators.exe", "ct-outils.exe") {
    $f = Join-Path $Dossier $exe
    if (-not (Test-Path $f)) { Write-Host "$exe : absent de $Dossier"; continue }
    # Horodatage si le serveur répond (la signature reste valable après l'expiration du certificat).
    $s = $null
    try {
        $s = Set-AuthenticodeSignature -FilePath $f -Certificate $cert -HashAlgorithm SHA256 `
            -TimestampServer "http://timestamp.digicert.com" -ErrorAction Stop
    } catch { $s = $null }
    if (-not $s -or $s.Status -ne "Valid") {
        $s = Set-AuthenticodeSignature -FilePath $f -Certificate $cert -HashAlgorithm SHA256
    }
    Write-Host ("{0} : {1} - éditeur {2}" -f $exe, $s.Status, $s.SignerCertificate.Subject)
}
