"""Pack Europe de Chevaliers Translators (0.17.1) : MiLMMT-46 4B, vocabulaire élagué aux langues
européennes, quantifié en IQ4_XS avec une matrice d'importance calculée sur des traductions entre
les 22 langues européennes, l'ukrainien et le serbe.

Étapes (toutes reproductibles, mêmes octets à chaque fois) :
  1. MiLMMT-46-4B-v1.0.Q8_0.gguf téléchargé depuis Hugging Face (empreinte SHA-256 vérifiée) ;
  2. vocabulaire élagué (outils/elaguer_vocab.py) : 180 852 jetons sur 262 208 gardés, ceux des
     écritures latine, grecque et cyrillique, de la ponctuation et des émojis ; un texte européen
     se découpe exactement comme avant ;
  3. llama-quantize (llama.cpp b12211) : IQ4_XS, matrice d'importance outils/packs/, matrice des
     jetons en q4_K ;
  4. empreinte du résultat comparée à outils/packs/europe.sha256.

La matrice d'importance est passée par son nom seul, depuis outils/packs/ : llama-quantize recopie
ce chemin dans le fichier produit, qui reste ainsi le même octet pour octet sur tout PC.

python outils/pack_europe.py <dossier de travail> <dossier des binaires llama.cpp> [--sans-source]
  --sans-source : supprime le Q8_0 téléchargé une fois élagué (place disque de GitHub Actions).
"""
import hashlib, os, subprocess, sys, urllib.request

ROOT = os.path.dirname(os.path.abspath(__file__))
WORK, BIN = sys.argv[1], sys.argv[2]
SRC_URL = "https://huggingface.co/mradermacher/MiLMMT-46-4B-v1.0-GGUF/resolve/3d05b58ace3d89a89af0574cdc49c84567f33969/MiLMMT-46-4B-v1.0.Q8_0.gguf"
SRC_SHA = "92796e263e22461c273c9f964ba4c1454d8dc764a482af60bc1ad9c269d9e7d0"
OUT = "MiLMMT-46-4B-v1.0-europe.IQ4_XS.gguf"
PACKS = os.path.join(ROOT, "packs")
IMATRIX = "milmmt-46-4b-europe.imatrix.gguf"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""):
            h.update(b)
    return h.hexdigest()


os.makedirs(WORK, exist_ok=True)
src = os.path.join(WORK, "MiLMMT-46-4B-v1.0.Q8_0.gguf")
if not os.path.exists(src) or sha256(src) != SRC_SHA:
    print("téléchargement :", SRC_URL, flush=True)
    urllib.request.urlretrieve(SRC_URL, src)
assert sha256(src) == SRC_SHA, "fichier source inattendu"
pruned = os.path.abspath(os.path.join(WORK, "MiLMMT-46-4B-v1.0-europe.Q8_0.gguf"))
subprocess.run([sys.executable, os.path.join(ROOT, "elaguer_vocab.py"), src, pruned], check=True)
# Fichier élagué de référence (22/11/2026, gguf 0.19.0) : 3 906 988 096 octets.
PRUNED_SHA = "0cef9dfd7de93062815dbe3849dce65662cddf2a6403414749c99a0e6f34ddce"
pruned_sha = sha256(pruned)
print("élagué :", os.path.getsize(pruned), "octets, SHA-256", pruned_sha,
      "(conforme)" if pruned_sha == PRUNED_SHA else f"(DIFFÉRENT de la référence {PRUNED_SHA})", flush=True)
if "--sans-source" in sys.argv:
    os.remove(src)
out = os.path.abspath(os.path.join(WORK, OUT))
subprocess.run([os.path.abspath(os.path.join(BIN, "llama-quantize")), "--allow-requantize", "--imatrix", IMATRIX,
                "--token-embedding-type", "q4_k", pruned, out, "IQ4_XS"], check=True, cwd=PACKS)
digest, size = sha256(out), os.path.getsize(out)
print(f"{OUT} : {size} octets, SHA-256 {digest}")
want = os.path.join(PACKS, "europe.sha256")
if os.path.exists(want):
    expected = open(want).read().split()[0]
    assert digest == expected, f"empreinte {digest}, attendue {expected}"
    print("empreinte conforme")
