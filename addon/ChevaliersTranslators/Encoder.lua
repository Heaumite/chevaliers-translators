-- Chevaliers Translators
-- Encoder.lua : logique pure du canal pixel CT-Pixel v1 (aucune API d'interface WoW).
-- Partagé avec le harnais de test hors jeu : ne dépend que de `bit` et de `math`.

local _, CT = ...
CT = CT or {}

local bit = bit
local band, bxor, rshift = bit.band, bit.bxor, bit.rshift
local floor, ceil, min, max = math.floor, math.ceil, math.min, math.max
local strbyte = string.byte

local U32 = 4294967296
-- Normalise un résultat de `bit` en entier non signé 32 bits
-- (la bibliothèque de WoW et LuaJIT ne renvoient pas le même signe).
local function u32(x) return x % U32 end

CT.PROTO_VERSION = 1
CT.FRAME_TEST = 1
CT.FRAME_CHAT = 2
CT.FRAME_PING = 3

-- Blocs fixes, lus depuis le coin bas-droit vers la gauche :
-- 4 blocs de synchronisation (magenta, vert, magenta, vert),
-- 4 blocs d'étalonnage (gris niveaux 0 à 3), 2 blocs donnant le nombre de colonnes.
CT.SYNC = { 51, 12, 51, 12 }
CT.CALIB = { 0, 21, 42, 63 }
CT.FIXED_BLOCKS = 10
CT.HEADER_BYTES = 5 -- version, type, séquence (2), longueur
CT.CRC_BYTES = 4

-- CRC-32 IEEE (polynôme réfléchi 0xEDB88320), identique côté Rust.
local CRC = {}
for i = 0, 255 do
  local c = i
  for _ = 1, 8 do
    if c % 2 == 1 then
      c = u32(bxor(rshift(c, 1), 0xEDB88320))
    else
      c = u32(rshift(c, 1))
    end
  end
  CRC[i] = c
end

function CT.Crc32(bytes, first, last)
  local crc = 0xFFFFFFFF
  for i = first, last do
    crc = u32(bxor(rshift(crc, 8), CRC[band(bxor(crc, bytes[i]), 0xFF)]))
  end
  return u32(bxor(crc, 0xFFFFFFFF))
end

-- Capacité d'un bandeau de `cols` colonnes sur 2 rangées.
-- Retourne : symboles de données, octets, charge utile maximale.
function CT.Capacity(cols)
  local symbols = 2 * cols - CT.FIXED_BLOCKS
  local bytes = floor(symbols * 6 / 8)
  local payload = bytes - CT.HEADER_BYTES - CT.CRC_BYTES
  if payload > 255 then payload = 255 end
  return symbols, bytes, payload
end

-- Charge utile de test déterministe : le décodeur la recalcule pour détecter
-- toute corruption que le CRC aurait laissé passer.
local function lcg(s) return (s * 1664525 + 1013904223) % U32 end

function CT.TestPayload(seq, maxLen, out)
  local len = (seq * 37) % (maxLen + 1)
  local s = (seq * 2654435761 + 2654435769) % U32
  for i = 1, len do
    s = lcg(s)
    out[i] = floor(s / 16777216)
  end
  return len
end

-- Construit les 2*cols symboles (codes 0..63) d'une trame.
-- Ordre des blocs : rangée du bas de droite à gauche, puis rangée du haut de droite à gauche.
function CT.BuildBanner(ftype, seq, payload, plen, cols, syms, scratch)
  local nsym, _, maxp = CT.Capacity(cols)
  if plen > maxp then return false, "charge utile trop longue" end

  local b = scratch or {}
  b[1] = CT.PROTO_VERSION
  b[2] = ftype
  b[3] = floor(seq / 256) % 256
  b[4] = seq % 256
  b[5] = plen
  for i = 1, plen do b[5 + i] = payload[i] end
  local n = 5 + plen
  local crc = CT.Crc32(b, 1, n)
  b[n + 1] = floor(crc / 16777216) % 256
  b[n + 2] = floor(crc / 65536) % 256
  b[n + 3] = floor(crc / 256) % 256
  b[n + 4] = crc % 256
  n = n + 4

  for i = 1, 4 do syms[i] = CT.SYNC[i] end
  for i = 1, 4 do syms[4 + i] = CT.CALIB[i] end
  syms[9] = floor(cols / 64) % 64
  syms[10] = cols % 64

  local acc, bits, si = 0, 0, CT.FIXED_BLOCKS
  for i = 1, n do
    acc = acc * 256 + b[i]
    bits = bits + 8
    while bits >= 6 do
      bits = bits - 6
      local p = 2 ^ bits
      local s = floor(acc / p)
      acc = acc - s * p
      si = si + 1
      syms[si] = s
    end
  end
  if bits > 0 then
    si = si + 1
    syms[si] = acc * 2 ^ (6 - bits)
  end
  for i = si + 1, CT.FIXED_BLOCKS + nsym do syms[i] = 0 end
  return true
end

-- Codes de canal transmis dans les trames de chat.
CT.FRAME_SEND = 4
CT.FRAME_RETARGET = 5
-- Chat miroir (S5) : description d'une fenêtre de chat, et texte d'une ligne.
CT.FRAME_MVIEW = 6
CT.FRAME_MLINE = 7
-- Mesures de la police faites dans le jeu (avances, crénage, retrait des lignes coupées).
CT.FRAME_MADV = 8
-- Images par seconde du jeu (test de performance des modèles de traduction).
CT.FRAME_PERF = 9
-- Demande du joueur à l'application (« test 4b », « test stop »...).
CT.FRAME_CMD = 10
-- Signe de vie du chat miroir (déconnexion, chargement : l'application cache le miroir).
CT.FRAME_ALIVE = 11
-- État de l'envoi traduit (0.11) : boîte rouverte et prête pour le collage, annulé, parti.
CT.FRAME_STATE = 12
-- Rectangles des bulles d'aide, menus et fenêtres du jeu posés sur le chat (0.11) : le miroir
-- ne dessine rien dessous.
CT.FRAME_OCCLUDE = 13
-- Ligne de diagnostic de l'addon, recopiée dans la console de l'application (0.11).
CT.FRAME_NOTE = 14
-- Icône citée par une ligne du chat (0.12.3) : atlas (fichier, coordonnées, taille) ou chemin de
-- texture et son numéro de fichier, pour que l'application dessine la vraie image.
CT.FRAME_ICON = 15
-- Zone (0.13.3) : carte et canaux géographiques (Général, Commerce...) pour Trad:AUTO.
CT.FRAME_ZONE = 16
-- Message reçu pendant le verrouillage du chat (combat en instance), relayé après (0.13.3).
CT.FRAME_LATE = 17

CT.CHANNEL_CODES = {
  SAY = 1, YELL = 2, WHISPER = 3, WHISPER_INFORM = 4,
  PARTY = 5, PARTY_LEADER = 6, RAID = 7, RAID_LEADER = 8, RAID_WARNING = 9,
  GUILD = 10, OFFICER = 11, INSTANCE_CHAT = 12, INSTANCE_CHAT_LEADER = 13,
  CHANNEL = 14, EMOTE = 15,
  -- 0.12 : tous les messages écrits par des joueurs.
  BN_WHISPER = 16, BN_WHISPER_INFORM = 17, GUILD_DISCORD = 18, COMMUNITIES_CHANNEL = 19,
  VOICE_TEXT = 20, AFK = 21, DND = 22, BN_BROADCAST = 23,
}
-- Canaux numérotés (Général, Commerce, Services...) : 64 + numéro du canal, pour que chacun ait sa
-- langue majoritaire (Trad:AUTO).
CT.CHANNEL_NUMBERED = 64

-- Découpe « expéditeur\31texte » en fragments.
-- En-tête de fragment : identifiant (2 octets), numéro, total, canal.
function CT.ChatFragments(msgId, chanCode, sender, text, maxp, emit)
  local body = sender .. "\031" .. text
  local per = maxp - 5
  if per < 1 then return 0 end
  local total = #body
  local count = max(1, ceil(total / per))
  if count > 255 then
    count = 255
    total = 255 * per
  end
  for part = 1, count do
    local p = {
      floor(msgId / 256) % 256, msgId % 256, part, count, chanCode,
    }
    local len = 5
    for i = (part - 1) * per + 1, min(part * per, total) do
      len = len + 1
      p[len] = strbyte(body, i)
    end
    emit(p, len)
  end
  return count
end

return CT
