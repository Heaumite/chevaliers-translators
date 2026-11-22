"""Élague le vocabulaire d'un modèle Gemma 3 (GGUF) aux langues européennes (22/11/2026).

On garde : les jetons de contrôle et spéciaux, les 256 octets de repli, et tout jeton dont le texte
n'utilise que les écritures latine, grecque et cyrillique, les chiffres, la ponctuation et les
symboles courants. Un texte européen se découpe donc exactement comme avant (tous ses morceaux
sont gardés) ; le modèle ne peut simplement plus écrire de chinois, d'arabe, de thaï... Les lignes
gardées de la matrice des jetons sont recopiées octet pour octet : rien n'est requantifié.

python outils/elaguer_vocab.py entree.gguf sortie.gguf   (gguf 0.19.0, numpy 2.5.5)
"""
import sys
import unicodedata

import numpy as np
from gguf import GGUFReader, GGUFWriter, GGUFValueType, TokenType

SRC, DST = sys.argv[1], sys.argv[2]

ALLOWED_RANGES = [
    (0x0000, 0x024F),   # latin de base, latin-1, latin étendu A et B
    (0x0250, 0x02FF),   # API, lettres modificatives
    (0x0300, 0x036F),   # diacritiques
    (0x0370, 0x03FF),   # grec
    (0x0400, 0x052F),   # cyrillique et supplément
    (0x1E00, 0x1EFF),   # latin étendu additionnel
    (0x1F00, 0x1FFF),   # grec étendu
    (0x2000, 0x206F),   # ponctuation générale
    (0x20A0, 0x20CF),   # monnaies
    (0x2100, 0x214F),   # symboles de lettres (№, ℃...)
    (0x2190, 0x21FF),   # flèches
    (0x2200, 0x22FF),   # opérateurs mathématiques
    (0x2460, 0x24FF),   # chiffres cerclés
    (0x2500, 0x25FF),   # traits, formes
    (0x2600, 0x27BF),   # symboles divers, dingbats
    (0xFB00, 0xFB06),   # ligatures latines
    (0xFE00, 0xFE0F),   # sélecteurs de variante (émojis)
    (0x1F000, 0x1FAFF), # émojis et pictogrammes (courants dans le chat)
]


def allowed(ch):
    cp = ord(ch)
    return any(a <= cp <= b for a, b in ALLOWED_RANGES)


r = GGUFReader(SRC)
f_tokens = r.fields["tokenizer.ggml.tokens"]
tokens = [bytes(f_tokens.parts[i]).decode("utf-8", errors="replace") for i in f_tokens.data]
types = [int(r.fields["tokenizer.ggml.token_type"].parts[i][0]) for i in r.fields["tokenizer.ggml.token_type"].data]
scores = [float(r.fields["tokenizer.ggml.scores"].parts[i][0]) for i in r.fields["tokenizer.ggml.scores"].data]

keep = []
for i, (t, ty) in enumerate(zip(tokens, types)):
    if ty in (TokenType.CONTROL, TokenType.BYTE, TokenType.UNKNOWN):
        keep.append(i)
        continue
    if ty == TokenType.USER_DEFINED:
        # Jetons ajoutés (<start_of_turn>, balises...) : gardés s'ils sont lisibles en latin.
        if all(allowed(c) for c in t):
            keep.append(i)
        continue
    text = t.replace("▁", " ")
    if text and all(allowed(c) for c in text):
        keep.append(i)
remap = {old: new for new, old in enumerate(keep)}
print(f"jetons gardés : {len(keep)} sur {len(tokens)}")

w = GGUFWriter(DST, arch=r.fields["general.architecture"].parts[r.fields["general.architecture"].data[0]].tobytes().decode())
SKIP = {"general.architecture", "GGUF.version", "GGUF.tensor_count", "GGUF.kv_count",
        "tokenizer.ggml.tokens", "tokenizer.ggml.scores", "tokenizer.ggml.token_type"}
ID_KEYS = {"tokenizer.ggml.bos_token_id", "tokenizer.ggml.eos_token_id", "tokenizer.ggml.unknown_token_id",
           "tokenizer.ggml.padding_token_id", "tokenizer.ggml.eot_token_id", "tokenizer.ggml.mask_token_id"}
for k, f in r.fields.items():
    if k in SKIP:
        continue
    vt = f.types[0]
    if vt == GGUFValueType.ARRAY:
        sub = f.types[1]
        if sub == GGUFValueType.STRING:
            w.add_array(k, [bytes(f.parts[i]).decode("utf-8") for i in f.data])
        else:
            w.add_array(k, [f.parts[i].tolist()[0] for i in f.data])
    elif vt == GGUFValueType.STRING:
        w.add_string(k, bytes(f.parts[f.data[0]]).decode("utf-8"))
    else:
        v = f.parts[f.data[0]].tolist()[0]
        if k in ID_KEYS:
            v = remap.get(v, 0)
        w.add_key_value(k, v, vt)
w.add_token_list([tokens[i] for i in keep])
w.add_token_scores([scores[i] for i in keep])
w.add_token_types([types[i] for i in keep])

idx = np.array(keep, dtype=np.int64)
for t in r.tensors:
    data = t.data
    shape = [int(x) for x in t.shape]
    if t.name == "token_embd.weight":
        # Une ligne par jeton (dernière dimension GGML) : les lignes gardées, octet pour octet.
        data = np.ascontiguousarray(data[idx])
        shape[-1] = len(keep)
    w.add_tensor(t.name, data, raw_dtype=t.tensor_type)
w.write_header_to_file()
w.write_kv_data_to_file()
w.write_tensors_to_file()
w.close()
print("écrit :", DST)
