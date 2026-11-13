-- Chevaliers Translators
-- Mirror.lua : chat miroir (lignes des joueurs cachées, redessinées traduites par l'application).
-- Piloté par la traduction du chat (/chevaliers on|off) ; outils de développement :
--   /chevaliers miroir on           texte des lignes de joueurs caché, l'application le redessine
--   /chevaliers miroir calage       texte officiel visible, l'application dessine par-dessus en magenta
--   /chevaliers miroir off          tout redevient normal (aussi après /reload)
--   /chevaliers miroir sync         renvoie tout à l'application (si elle a été relancée)
--   /chevaliers miroir echantillon  quelques lignes de test dans la fenêtre principale
--
-- Bulles d'aide, menus, fenêtres de réglage et fenêtres du jeu posés sur une fenêtre de chat :
-- leurs rectangles partent à l'application, qui ne dessine rien dessous (0.11).
--
-- L'addon ne modifie que l'opacité du conteneur des lignes de chaque fenêtre
-- (FontStringContainer) : fond, onglets, défilement, liens, saisie restent ceux de Blizzard.
-- Il décrit ce que chaque fenêtre affiche : position, police, lignes visibles et leur âge
-- (pour l'effacement progressif). Le texte officiel réapparaît sous verrouillage du chat,
-- et dès qu'une ligne visible est « secrète » (illisible pour un addon).
-- Le journal de combat n'est jamais modifié.

local ADDON, CT = ...
local floor, ceil, format, min, max = math.floor, math.ceil, string.format, math.min, math.max
local strbyte = string.byte

local FLAG_VISIBLE, FLAG_CALAGE, FLAG_OFFICIAL, FLAG_FADE, FLAG_TRUNC = 1, 2, 4, 8, 16
-- 0.12.3 : chaque ligne porte son nombre de lignes à l'écran ; numéro 0 = ligne dessinée par WoW.
local FLAG_ROWS = 32
-- 0.14, dispositif télévision : texte du jeu laissé visible, recouvert par le chat traduit.
local FLAG_COVER = 64
-- 0.14.4 : hauteur réelle de chaque ligne (une icône dans le texte, comme la qualité d'un objet,
-- agrandit la ligne chez WoW) : écart à la hauteur théorique, un octet par ligne.
local FLAG_HEIGHTS = 128
local function HasFlag(flags, bit) return floor(flags / bit) % 2 == 1 end
local POLL = 0.05        -- lecture des fenêtres : 20 fois par seconde
local MIN_INTERVAL = 0.1 -- au plus 10 descriptions par seconde et par fenêtre
local REFRESH = 15       -- description renvoyée même sans changement (0.13.3 : 15 s au lieu de 3, bandeau immobile au repos)

-- Texte d'une ligne visible renvoyé en entier (application relancée, trame manquée). 20 s avant la
-- 0.12.3 : sous un chat très actif, ces renvois occupaient le bandeau.
local TEXT_RESEND = 60
-- Messages relayés récemment : une ligne neuve qui en contient un le cite par son numéro
-- (« \029XXXX\029 ») au lieu de le renvoyer (0.12.3 : moitié moins de trames par message).
local CITE_TTL, CITE_MIN, CITE_MAX = 20, 16, 40
local OCC_POLL = 0.1     -- rectangles posés sur le chat : relus 10 fois par seconde

local M = {
  mode = "off",
  ids = setmetatable({}, { __mode = "k" }), -- messageInfo -> identifiant de ligne
  texts = {},                               -- identifiant -> texte déjà envoyé
  textsAt = {},                             -- identifiant -> moment de l'envoi
  chatRects = {},                           -- index de fenêtre -> rectangle en pixels (fenêtre reflétée)
  nextId = 0,
  state = {},                               -- index de fenêtre -> dernier envoi
  stats = { views = 0, lines = 0, official = 0, truncated = 0 },
}
CT.Mirror = M

local function IsSecret(v) return issecretvalue ~= nil and issecretvalue(v) or false end

local cites = {}  -- { id, text, at }, du plus récent au plus ancien

function M:NoteChat(id, text)
  if type(text) ~= "string" or #text < CITE_MIN then return end
  table.insert(cites, 1, { id = id, text = text, at = GetTime() })
  if #cites > CITE_MAX then cites[#cites] = nil end
end

-- Ligne envoyée la première fois : le message relayé qu'elle contient est cité par son numéro.
local function Cite(text, now)
  for _, c in ipairs(cites) do
    if now - c.at > CITE_TTL then break end
    local s, e = text:find(c.text, 1, true)
    if s then
      return text:sub(1, s - 1) .. "\029" .. format("%04X", c.id) .. "\029" .. text:sub(e + 1)
    end
  end
  return text
end

local function U8(t, v) t[#t + 1] = floor(v) % 256 end
local function U16(t, v)
  v = floor(v + 0.5)
  if v < 0 then v = 0 elseif v > 65535 then v = 65535 end
  t[#t + 1] = floor(v / 256)
  t[#t + 1] = v % 256
end
local function I8(t, v)
  v = floor(v + 0.5)
  if v < -127 then v = -127 elseif v > 127 then v = 127 end
  t[#t + 1] = v % 256
end

-- Fenêtres de chat Blizzard à refléter (liste relue toutes les 2 s : fenêtres temporaires).
local frameList, frameListAt = {}, -10
local function ChatFrames(now)
  if now - frameListAt < 2 then return frameList end
  frameListAt = now
  frameList = {}
  local C = Constants and Constants.ChatFrameConstants
  local count = (C and C.MaxChatWindows or NUM_CHAT_WINDOWS or 10) + 10
  for i = 1, count do
    local cf = _G["ChatFrame" .. i]
    if type(cf) == "table" and cf.FontStringContainer and cf.visibleLines
      and not (IsCombatLog and IsCombatLog(cf)) then
      frameList[#frameList + 1] = { i = i, cf = cf }
    end
  end
  return frameList
end

local function LineId(info)
  local id = M.ids[info]
  if not id then
    M.nextId = M.nextId % 65535 + 1
    id = M.nextId
    M.ids[info] = id
    M.texts[id] = nil
    M.textsAt[id] = nil
    if id % 1000 == 0 then
      -- Oubli des textes trop anciens pour être encore affichés.
      for old in pairs(M.texts) do
        if (id - old) % 65535 > 5000 then M.texts[old], M.textsAt[old] = nil, nil end
      end
    end
  end
  return id
end

-- Lignes des joueurs (dire, chuchotements, groupe, raid, guilde, canaux...) : traduites par
-- l'application. Depuis la 0.14.5, les autres lignes lisibles (butin, artisanat, monnaies,
-- système, créatures, messages des addons...) sont aussi redessinées, sans traduction : une
-- traduction plus longue que l'original prend alors les lignes qu'il lui faut et les plus
-- anciennes remontent, comme dans WoW (avant : coupée par « … » entre deux lignes du jeu). Seules
-- les lignes secrètes (instances, combat) restent dessinées par WoW.
local PLAYER_TYPES = {
  SAY = true, YELL = true, EMOTE = true, WHISPER = true, WHISPER_INFORM = true,
  BN_WHISPER = true, BN_WHISPER_INFORM = true, PARTY = true, PARTY_LEADER = true,
  RAID = true, RAID_LEADER = true, RAID_WARNING = true, GUILD = true, OFFICER = true,
  GUILD_DISCORD = true, INSTANCE_CHAT = true, INSTANCE_CHAT_LEADER = true,
  COMMUNITIES_CHANNEL = true, VOICE_TEXT = true, AFK = true, DND = true,
  BN_INLINE_TOAST_BROADCAST = true, CHANNEL = true,
}
local typeById, typeByIdAt = {}, -100

-- Type de message d'après le numéro rangé par la fenêtre de chat (ChatTypeInfo[type].id), si
-- ces numéros distinguent bien les types (0.12.4 : sur retail 12.1.5, ils ne suffisaient pas).
local function ChatTypeOf(id, now)
  if type(id) ~= "number" or IsSecret(id) then return nil end
  local t = typeById[id]
  if t == nil and now - typeByIdAt > 10 and type(ChatTypeInfo) == "table" then
    typeByIdAt = now
    typeById = {}
    local count, seen, distinct = 0, {}, 0
    for name, v in pairs(ChatTypeInfo) do
      if type(v) == "table" and type(v.id) == "number" then
        count = count + 1
        if not seen[v.id] then seen[v.id], distinct = true, distinct + 1 end
        typeById[v.id] = name
      end
    end
    if distinct < count then typeById = {} end
    t = typeById[id]
  end
  return t
end

-- Ligne écrite par un joueur ? La fenêtre de chat range, pour ces messages, l'événement d'origine
-- (« CHAT_MSG_SAY ») en 4e donnée de la ligne ; les messages du jeu (système, butin...) n'en ont
-- pas, ceux des créatures ont le leur (« CHAT_MSG_MONSTER_SAY »).
local oursCache = setmetatable({}, { __mode = "k" })
local function Ours(info, now)
  local o = oursCache[info]
  if o == nil then
    o = false
    local ed = info.extraData
    if type(ed) == "table" and not IsSecret(ed) then
      local ev = ed[4]
      if type(ev) == "string" and not IsSecret(ev) then
        o = CT.CHAT_EVENTS ~= nil and CT.CHAT_EVENTS[ev] ~= nil
      else
        local t = ChatTypeOf(ed[1], now)
        o = type(t) == "string" and (PLAYER_TYPES[t] or t:match("^CHANNEL%d+$") ~= nil) or false
      end
    end
    oursCache[info] = o
  end
  return o
end
M.Ours = Ours

-- Opacité ligne par ligne : WoW la recalcule (affichage, fondu) ; juste après, les lignes des
-- joueurs sont remises à 0 (l'application les dessine).
local hideLines = setmetatable({}, { __mode = "k" })
local hooked = setmetatable({}, { __mode = "k" })

local function ApplyLinesNow(cf)
  if not hideLines[cf] then return end
  -- Chat verrouillé ou combat : rien n'est caché (le miroir laisse tout le texte officiel).
  if CT.InLockdown() or (CT.InCombat and CT.InCombat()) then return end
  local now = GetTime()
  for _, fs in ipairs(cf.visibleLines) do
    local info = fs.messageInfo
    -- Toute ligne lisible : redessinée par l'application (0.14.5).
    if info and not IsSecret(info.message) and fs:GetAlpha() ~= 0 then fs:SetAlpha(0) end
  end
end

-- Appelé après le rafraîchissement du chat par WoW : jamais d'erreur affichée (valeurs secrètes).
local function ApplyLines(cf)
  pcall(ApplyLinesNow, cf)
end

-- Retour au texte officiel : opacité que WoW donnerait (fondu compris).
local function RestoreLines(cf)
  local now = GetTime()
  local fade = cf.CanEffectivelyFade and cf:CanEffectivelyFade()
  for _, fs in ipairs(cf.visibleLines) do
    local info = fs.messageInfo
    if info then
      local a = 1
      local ts = info.timestamp
      if IsSecret(ts) then ts = nil end
      if fade and cf.CalculateLineAlphaValueFromTimestamp then
        local ok, v = pcall(cf.CalculateLineAlphaValueFromTimestamp, cf, now,
          max(tonumber(ts) or 0, tonumber(cf.overrideFadeTimestamp) or 0))
        if ok and type(v) == "number" and not IsSecret(v) then a = v end
      end
      pcall(function() if fs:GetAlpha() ~= a then fs:SetAlpha(a) end end)
    end
  end
end

-- Vrai si la fenêtre se prête au réglage ligne par ligne (sinon : tout le texte, comme avant).
-- Une fenêtre jamais affichée n'a pas encore de lignes : réponse remise à plus tard.
local function PerLine(cf)
  if hooked[cf] == nil then
    local first = cf.visibleLines[1]
    if type(first) ~= "table" then return false end
    hooked[cf] = false
    if hooksecurefunc and type(cf.RefreshDisplay) == "function" and type(cf.UpdateFading) == "function"
      and first.SetAlpha then
      hooksecurefunc(cf, "RefreshDisplay", ApplyLines)
      hooksecurefunc(cf, "UpdateFading", ApplyLines)
      hooked[cf] = true
    end
  end
  return hooked[cf]
end

local function SetOfficialHidden(cf, hide)
  local c = cf.FontStringContainer
  if PerLine(cf) then
    if c and c:GetAlpha() ~= 1 then c:SetAlpha(1) end
    if hide then
      hideLines[cf] = true
      ApplyLines(cf)
    elseif hideLines[cf] then
      hideLines[cf] = nil
      RestoreLines(cf)
    end
    return
  end
  local want = hide and 0 or 1
  if c and c:GetAlpha() ~= want then c:SetAlpha(want) end
end

-- Nombre de lignes d'une ligne de chat, nil si inconnu. En instance (12.x), les mesures des
-- lignes peuvent être des valeurs secrètes : ni comparables ni calculables (0.13.3 : erreurs
-- « attempt to compare local 'r' (a secret number value) » en donjon et en raid).
local function NumLines(fs)
  if not fs.GetNumLines then return nil end
  local ok, r = pcall(fs.GetNumLines, fs)
  if not ok or IsSecret(r) or type(r) ~= "number" then return nil end
  return r
end

-- Nombre de lignes à l'écran d'un message (coupé par WoW).
local function Rows(fs)
  local r = NumLines(fs)
  if r and r >= 1 then return min(15, r) end
  return 1
end

-- Écart entre la hauteur réelle d'une ligne de chat et sa hauteur théorique (lignes à l'écran,
-- hauteur de ligne, espacement), en quarts de pixel (−127..127) ; −128 : inconnu (valeur secrète).
-- Une icône plus haute que le texte (qualité d'un objet fabriqué, atlas) agrandit la ligne : sans
-- cet écart, l'application plaçait les messages des joueurs sur les lignes de WoW (0.14.3).
local function HeightDelta(fs, rows, lineH, spacing, k)
  if not fs.GetHeight then return -128 end
  local ok, hgt = pcall(fs.GetHeight, fs)
  if not ok or IsSecret(hgt) or type(hgt) ~= "number" or hgt <= 0 then return -128 end
  local d = floor((hgt - (rows * lineH + (rows - 1) * spacing)) * k * 4 + 0.5)
  return max(-127, min(127, d))
end

local function FontFlagsCode(flags)
  flags = flags or ""
  local code = 0
  if flags:find("THICK") then code = code + 2 elseif flags:find("OUTLINE") then code = code + 1 end
  if flags:find("MONOCHROME") then code = code + 4 end
  return code
end

-- Style lu sur une ligne réelle (même police que la fenêtre), sinon sur la fenêtre.
local function Style(cf)
  local fs = cf.visibleLines[1]
  local src = (fs and fs.GetFont) and fs or cf
  local path, size, flags = src:GetFont()
  local lineH = (fs and fs.GetLineHeight) and fs:GetLineHeight() or size
  local spacing = (src.GetSpacing and src:GetSpacing()) or 0
  local sx, sy = 0, 0
  if src.GetShadowOffset then sx, sy = src:GetShadowOffset() end
  local sa = 0
  if src.GetShadowColor then sa = select(4, src:GetShadowColor()) or 0 end
  if type(path) == "number" then path = "FDID:" .. path end
  return tostring(path or ""), size or 14, flags, lineH or size or 14, spacing, sx or 0, sy or 0, sa
end

local function ColorHex(info, cf)
  local r, g, b = info.r, info.g, info.b
  if IsSecret(r) or IsSecret(g) or IsSecret(b) or not (r and g and b) then
    r, g, b = cf:GetTextColor()
  end
  local function c(v) return min(255, max(0, floor((tonumber(v) or 1) * 255 + 0.5))) end
  return format("%02X%02X%02X", c(r), c(g), c(b))
end

local lines = {}

local function Describe(i, cf, now)
  local active = M.mode ~= "off" and cf:IsVisible()
  local flags, n, secret, sig = 0, 0, false, 0
  local view
  if active then
    local l, b, w, h = cf:GetLeft(), cf:GetBottom(), cf:GetWidth(), cf:GetHeight()
    if IsSecret(l) or IsSecret(b) or IsSecret(w) or IsSecret(h) or not (l and b and w and h) then active = false end
    if active then
      local _, ph = GetPhysicalScreenSize()
      local k = cf:GetEffectiveScale() * (ph or 768) / 768
      local path, size, fflags, lineH, spacing, sx, sy, sa = Style(cf)
      local canFade = cf.CanEffectivelyFade and cf:CanEffectivelyFade()
      local override = cf.overrideFadeTimestamp or 0
      local perLine = PerLine(cf)
      -- Chat verrouillé (combat en instance) ou joueur en combat : tout le texte officiel, rien
      -- n'est redessiné ; les traductions reviennent hors combat.
      local locked = CT.InLockdown() or (CT.InCombat and CT.InCombat())
      for _, fs in ipairs(cf.visibleLines) do
        if locked then secret = true break end
        local info = fs.messageInfo
        if not info then break end
        local hidden = IsSecret(info.message)
        -- Ligne secrète hors verrouillage (reçue pendant un combat) : laissée à WoW comme les
        -- lignes du jeu (0.13.3) ; sans le mode ligne par ligne, toute la fenêtre reste officielle.
        if hidden and not perLine then secret = true break end
        n = n + 1
        local e = lines[n] or {}
        lines[n] = e
        -- Ligne secrète : laissée à WoW, seule sa place est décrite. Les autres sont redessinées,
        -- celles des joueurs traduites (0.14.5).
        e.gap = hidden
        e.ours = not hidden and Ours(info, now)
        local ts = info.timestamp
        if IsSecret(ts) then ts = nil end
        e.info, e.base, e.rows = info, max(tonumber(ts) or 0, override), Rows(fs)
        e.hd = HeightDelta(fs, e.rows, lineH, spacing, k)
        e.id = e.gap and 0 or LineId(info)
      end
      flags = FLAG_VISIBLE + FLAG_ROWS + FLAG_HEIGHTS
      if M.mode == "calage" then flags = flags + FLAG_CALAGE end
      if secret or CT.InLockdown() then flags = flags + FLAG_OFFICIAL end
      if canFade then flags = flags + FLAG_FADE end
      if CT.CoverMode and CT.CoverMode() then flags = flags + FLAG_COVER end
      view = {
        l = floor(l * k + 0.5), b = floor(b * k + 0.5), w = floor(w * k + 0.5), h = floor(h * k + 0.5),
        size = size * k, lineH = lineH * k, spacing = spacing * k,
        sx = sx * k, sy = sy * k, sa = sa, fflags = FontFlagsCode(fflags), path = path,
        k = k,
        tv = cf.GetTimeVisible and cf:GetTimeVisible() or 120,
        fd = cf.GetFadeDuration and cf:GetFadeDuration() or 3,
      }
      -- Empreinte : change dès que l'affichage change (pas avec le temps qui passe).
      sig = flags * 7 + view.l * 3 + view.b * 5 + view.w * 11 + view.h * 13
        + floor(view.size * 10) * 17 + floor(view.lineH * 10) * 19 + view.fflags * 23 + #path * 29
      for j = 1, n do
        sig = (sig * 31 + lines[j].id * 37 + floor(lines[j].base * 10) + lines[j].rows * 41 + (lines[j].hd + 128) * 43) % 2147483647
      end
    end
  end
  local hide = active and M.mode == "on" and flags % 8 < FLAG_OFFICIAL and not HasFlag(flags, FLAG_COVER)
  SetOfficialHidden(cf, hide)
  M.chatRects[i] = (active and view and flags % 8 < FLAG_OFFICIAL) and { view.l, view.b, view.w, view.h } or nil
  return active, flags, n, view, sig
end

-- Mesures de la police dans le jeu --------------------------------------------
-- Pour couper et placer le texte exactement comme WoW, l'application reçoit l'avance
-- de chaque caractère affiché telle que WoW la calcule (taille, échelle et arrondis
-- compris), quelques paires pour savoir si WoW applique le crénage, et le retrait des
-- lignes coupées. Une seule mesure par caractère et par police.

local KERN_PAIRS = { "AV", "To", "Wa", "Ty", "Yo", "LT", "P.", "Av" }
local measureFrame, probe
local measured = {}   -- clé de police -> { chars = {}, kerned = bool, indent = nombre ou nil }

local function Codepoint(ch)
  local b1, b2, b3, b4 = strbyte(ch, 1, 4)
  b2, b3, b4 = b2 or 128, b3 or 128, b4 or 128
  if b1 < 0x80 then return b1 end
  if b1 < 0xE0 then return (b1 % 0x20) * 64 + b2 % 0x40 end
  if b1 < 0xF0 then return ((b1 % 0x10) * 64 + b2 % 0x40) * 64 + b3 % 0x40 end
  return (((b1 % 0x08) * 64 + b2 % 0x40) * 64 + b3 % 0x40) * 64 + b4 % 0x40
end

-- Texte réellement dessiné : sans couleurs, données de liens, icônes.
local function Displayed(s)
  s = s:gsub("||", "\001")
  s = s:gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|cn[^:]*:", ""):gsub("|r", "")
  s = s:gsub("|H.-|h", ""):gsub("|h", ""):gsub("|T.-|t", ""):gsub("|A.-|a", "")
  s = s:gsub("|K.-|k", "?"):gsub("|n", ""):gsub("|4([^:;]*)[^;]*;", "%1")
  return (s:gsub("\001", "|"))
end

local function SetupProbe(cf, path, size, fflags)
  if not measureFrame then
    measureFrame = CreateFrame("Frame", nil, UIParent)
    measureFrame:SetSize(1, 1)
    measureFrame:SetPoint("TOPLEFT")
    measureFrame:SetAlpha(0)
    probe = measureFrame:CreateFontString(nil, "BACKGROUND")
    probe:SetPoint("TOPLEFT")
  end
  -- Même échelle que la fenêtre de chat : même taille en pixels, mêmes arrondis.
  measureFrame:SetScale(cf:GetEffectiveScale() / UIParent:GetEffectiveScale())
  probe:SetFont(path, size, fflags or "")
end

local function Width(s)
  probe:SetText(s)
  if probe.GetUnboundedStringWidth then return probe:GetUnboundedStringWidth() end
  return probe:GetStringWidth()
end

-- Mesure les caractères nouveaux des lignes visibles ; renvoie les trames à envoyer.
local PAIR_BUDGET = 150 -- paires mesurées au plus par description (reste : description suivante)
local UTF8 = "[%z\1-\127\194-\244][\128-\191]*"

-- Retrait des lignes coupées, lu sur une vraie ligne du chat coupée en plusieurs lignes.
-- Renvoie le retrait, ou nil et vrai si une ligne coupée existait mais n'a rien donné.
local function IndentFromLines(cf, n)
  local tried = false
  for j = 1, n do
    local fs = cf.visibleLines[j]
    if fs and fs.CalculateScreenAreaFromCharacterSpan and (NumLines(fs) or 0) > 1
      and lines[j] and lines[j].info and not IsSecret(lines[j].info.message) then
      tried = true
      local count = 0
      for _ in Displayed(tostring(lines[j].info.message)):gmatch(UTF8) do count = count + 1 end
      -- Indices des caractères affichés, puis des octets du texte (selon la version du jeu).
      for _, last in ipairs({ count, #tostring(lines[j].info.message) }) do
        local ok, areas = pcall(fs.CalculateScreenAreaFromCharacterSpan, fs, 1, last)
        if ok and type(areas) == "table" and areas[2] and areas[2].left and areas[1] and areas[1].left then
          return areas[2].left - areas[1].left
        end
      end
    end
  end
  return nil, tried
end

local function Measure(cf, n, view, k)
  local fs = cf.visibleLines[1]
  local path, size, fflags = (fs and fs.GetFont and fs or cf):GetFont()
  local key = format("%s:%s:%s:%.4f", tostring(path), tostring(size), tostring(fflags), k)
  local m = measured[key]
  local fresh = m == nil
  if fresh then
    m = { chars = {}, pairs = {}, indentTries = 0 }
    measured[key] = m
  end
  local adv, kern, indent = {}, {}, nil
  local ok, err = pcall(function()
    SetupProbe(cf, path, size, fflags)
    local base = Width("II")
    local function Char(ch)
      local a = m.chars[ch]
      if not a then
        local rep = ch == "|" and "||" or ch
        a = (Width("I" .. rep:rep(10) .. "I") - base) / 10
        m.chars[ch] = a
        adv[#adv + 1] = { Codepoint(ch), a * k }
      end
      return a
    end
    -- Crénage d'une paire tel que WoW l'applique (envoyé seulement s'il n'est pas nul).
    local budget = PAIR_BUDGET
    local function Pair(a, b, always)
      local key2 = a .. b
      if m.pairs[key2] or budget <= 0 or a == "|" or b == "|" then return end
      budget = budget - 1
      local w = (Width("I" .. (key2 .. "I"):rep(5) .. "I") - base) / 5
      local v = w - Char(a) - Char(b) - Char("I")
      m.pairs[key2] = v
      if always or math.abs(v * k) >= 0.02 then
        kern[#kern + 1] = { Codepoint(a), Codepoint(b), v * k }
      end
    end
    Char("I")
    if fresh then
      for _, pair in ipairs(KERN_PAIRS) do Pair(pair:sub(1, 1), pair:sub(2, 2), true) end
    end
    for j = 1, n do
      local prev
      -- Lignes laissées à WoW : rien à mesurer.
      local text = lines[j].gap and "" or tostring(lines[j].info.message)
      for ch in Displayed(text):gmatch(UTF8) do
        Char(ch)
        if prev then Pair(prev, ch) end
        prev = ch
      end
    end
    if m.indent == nil and m.indentTries < 10 then
      local v, tried = IndentFromLines(cf, n)
      if v then
        m.indent = v
        indent = v * k
      elseif tried then
        -- Essai compté seulement quand une ligne coupée était visible.
        m.indentTries = m.indentTries + 1
      end
    end
  end)
  if not ok then M.stats.measureError = tostring(err) end
  M.stats.indent = m.indent and format("%.1f px", m.indent * k)
    or (m.indentTries >= 10 and "non mesurable dans cette version du jeu (estimation utilisée)"
      or "en attente d'une ligne coupée")
  return adv, kern, indent
end

local function SendMeasures(i, view, adv, kern, indent)
  local maxp = CT.MaxPayload()
  local function Frame(kind)
    local p = {}
    U8(p, i); U16(p, view.size * 10); U8(p, kind)
    return p
  end
  local function Flush(p) CT.Enqueue(CT.FRAME_MADV, p, #p) end
  local per = floor((maxp - 5) / 5)
  local j = 1
  while j <= #adv do
    local p = Frame(0)
    local count = min(per, #adv - j + 1)
    U8(p, count)
    for q = j, j + count - 1 do
      local cp, a = adv[q][1], adv[q][2]
      U8(p, floor(cp / 65536)); U8(p, floor(cp / 256)); U8(p, cp)
      U16(p, a * 64)
    end
    Flush(p)
    j = j + count
  end
  local perK = floor((maxp - 5) / 8)
  j = 1
  while j <= #kern do
    local p = Frame(1)
    local count = min(perK, #kern - j + 1)
    U8(p, count)
    for q = j, j + count - 1 do
      local e = kern[q]
      for r = 1, 2 do U8(p, floor(e[r] / 65536)); U8(p, floor(e[r] / 256)); U8(p, e[r]) end
      local v = floor(e[3] * 64 + 0.5)
      if v < -32768 then v = -32768 elseif v > 32767 then v = 32767 end
      U8(p, floor((v % 65536) / 256)); U8(p, v % 256)
    end
    Flush(p)
    j = j + count
  end
  if indent then
    local p = Frame(2)
    U16(p, indent * 64)
    Flush(p)
  end
  M.stats.measured = (M.stats.measured or 0) + #adv
end

-- Icônes des lignes (rang de qualité des objets d'artisanat, marques de raid...) : décrites à
-- l'application une fois, puis toutes les 2 min (application relancée entre-temps).
local ICON_RESEND = 120
local iconsSent = {}

local function SendIcon(kind, name, fid, l, r, t, b, w, h)
  name = name:sub(1, 90)
  local p = { kind, #name }
  for c = 1, #name do p[#p + 1] = strbyte(name, c) end
  fid = floor(tonumber(fid) or 0)
  if fid < 0 or fid > 4294967295 then fid = 0 end
  U16(p, floor(fid / 65536)); U16(p, fid % 65536)
  for _, v in ipairs({ l, r, t, b }) do
    v = tonumber(v) or 0
    U16(p, min(1, max(0, v)) * 65535)
  end
  U16(p, tonumber(w) or 0); U16(p, tonumber(h) or 0)
  CT.Enqueue(CT.FRAME_ICON, p, #p)
  M.stats.icons = (M.stats.icons or 0) + 1
end

local function AnnounceIcons(text, now)
  if not text:find("|[AT]") then return end
  for name in text:gmatch("|A:([^:|]+)") do
    if now - (iconsSent[name] or -1000) > ICON_RESEND then
      iconsSent[name] = now
      local ok, info = pcall(function() return C_Texture and C_Texture.GetAtlasInfo and C_Texture.GetAtlasInfo(name) end)
      if ok and type(info) == "table" then
        local fid = info.file
        if type(fid) ~= "number" and type(info.filename) == "string" and GetFileIDFromPath then
          local okF, f = pcall(GetFileIDFromPath, info.filename)
          fid = okF and f or 0
        end
        SendIcon(0, name, fid, info.leftTexCoord or 0, info.rightTexCoord or 1, info.topTexCoord or 0,
          info.bottomTexCoord or 1, info.width or 0, info.height or 0)
      end
    end
  end
  for path in text:gmatch("|T([^:|]+)") do
    if not tonumber(path) and now - (iconsSent[path] or -1000) > ICON_RESEND then
      iconsSent[path] = now
      if GetFileIDFromPath then
        local ok, fid = pcall(GetFileIDFromPath, path)
        if ok and tonumber(fid) then SendIcon(1, path, fid, 0, 1, 0, 1, 0, 0) end
      end
    end
  end
end

local function SendView(i, cf, now, flags, n, view)
  local maxp = CT.MaxPayload()
  -- Textes d'abord : la description qui les cite arrive ensuite.
  for j = 1, n do
    local e = lines[j]
    if not e.gap then
      local text = tostring(e.info.message)
      local fresh = M.texts[e.id] ~= text
      if fresh or now - (M.textsAt[e.id] or -100) > TEXT_RESEND then
        M.texts[e.id] = text
        M.textsAt[e.id] = now
        -- Renvoi périodique : texte entier (le message cité a pu manquer à l'application).
        local sent = fresh and Cite(text, now) or text
        if sent ~= text then M.stats.cited = (M.stats.cited or 0) + 1 end
        AnnounceIcons(text, now)
        -- Ligne du jeu (pas d'un joueur) : bit de poids fort du numéro de fenêtre (0.14.5).
        CT.ChatFragments(e.id, i + (e.ours and 0 or 128), ColorHex(e.info, cf), sent, maxp,
          function(p, len) CT.Enqueue(CT.FRAME_MLINE, p, len) end)
        M.stats.lines = M.stats.lines + 1
      end
    end
  end
  local p = {}
  U8(p, i)
  local path = view and view.path:sub(1, 120) or ""
  local fixed = 28 + #path
  -- Par ligne : numéro, âge et lignes à l'écran (4 octets), écart de hauteur (1 octet).
  local room = max(0, floor((maxp - fixed) / 5))
  if n > room then
    n = room
    flags = flags + FLAG_TRUNC
    M.stats.truncated = M.stats.truncated + 1
  end
  U8(p, flags)
  if view then
    U16(p, view.l); U16(p, view.b); U16(p, view.w); U16(p, view.h)
    U16(p, view.size * 10); U16(p, view.lineH * 10); U16(p, view.spacing * 10)
    I8(p, view.sx * 10); I8(p, view.sy * 10); U8(p, min(255, view.sa * 255 + 0.5)); U8(p, view.fflags)
    U16(p, view.tv * 10); U16(p, view.fd * 10)
    U8(p, #path)
    for c = 1, #path do p[#p + 1] = strbyte(path, c) end
    U8(p, n)
    for j = 1, n do
      -- Âge en dixièmes de seconde (12 bits) et nombre de lignes à l'écran (4 bits).
      local e = lines[j]
      local r = min(15, e.rows or 1) * 4096
      U16(p, e.id)
      U16(p, (e.gap and 0 or min(4095, max(0, floor((now - e.base) * 10)))) + r)
    end
    -- Pixels par unité de l'interface : taille des icônes (0.12.3).
    U16(p, view.k * 1000)
    -- Écart de hauteur de chaque ligne (0.14.4).
    for j = 1, n do U8(p, (lines[j].hd or -128) % 256) end
  end
  CT.Enqueue(CT.FRAME_MVIEW, p, #p)
  M.stats.views = M.stats.views + 1
  if view then
    local adv, kern, indent = Measure(cf, n, view, view.k)
    if #adv > 0 or #kern > 0 or indent then SendMeasures(i, view, adv, kern, indent) end
  end
end

function M:Poll(now)
  for _, e in ipairs(ChatFrames(now)) do
    local st = self.state[e.i] or { sig = -1, at = -100, active = false }
    self.state[e.i] = st
    -- Toute erreur de lecture (valeur secrète imprévue) laisse la fenêtre au texte officiel, sans
    -- message d'erreur répété à chaque lecture.
    local okD, active, flags, n, view, sig = pcall(Describe, e.i, e.cf, now)
    if not okD then
      M.stats.descError = tostring(active)
      SetOfficialHidden(e.cf, false)
      active, flags, n, view, sig = false, 0, 0, nil, -2
    end
    -- Compte les passages au texte officiel (verrouillage, ligne secrète), pas les lectures.
    local official = active and flags % 8 >= FLAG_OFFICIAL
    if official and not st.official then M.stats.official = M.stats.official + 1 end
    st.official = official
    local changed = sig ~= st.sig or active ~= st.active
    -- Fenêtre inactive : une seule description (pour effacer), pas de rappel périodique.
    local due = changed or (active and now - st.at >= REFRESH)
    if due and now - st.at >= MIN_INTERVAL then
      SendView(e.i, e.cf, now, flags, n, view)
      st.sig, st.at, st.active = sig, now, active
    end
  end
end

-- Fenêtres de chat reflétées (pour savoir si un message sera affiché).
function M:Frames()
  return ChatFrames(GetTime())
end

function M:Reset()
  iconsSent = {}
  self.texts = {}
  self.textsAt = {}
  self.state = {}
  self.chatRects = {}
  self.occReset = true
  frameListAt = -10
end

-- Fenêtres posées sur le chat ----------------------------------------------------
-- Rectangles (pixels physiques, origine en bas à gauche) des fenêtres du jeu affichées au-dessus
-- d'une fenêtre de chat reflétée : bulles d'aide (liens d'objets), menus (onglets du chat, bouton
-- Trad), réglages du chat, dialogues, sacs et fenêtres du jeu. Le miroir est dessiné par une
-- fenêtre posée sur le jeu : sans ces rectangles, il les recouvrirait.

local OCC_NAMES = {
  "GameTooltip", "ItemRefTooltip", "ShoppingTooltip1", "ShoppingTooltip2", "ItemRefShoppingTooltip1",
  "ItemRefShoppingTooltip2", "EmbeddedItemTooltip", "FriendsTooltip", "DropDownList1", "DropDownList2",
  "DropDownList3", "L_DropDownList1", "L_DropDownList2", "L_DropDownList3", "ChatMenu", "EmoteMenu",
  "LanguageMenu", "VoiceMacroMenu", "StaticPopup1", "StaticPopup2", "StaticPopup3", "StaticPopup4",
  "ChatConfigFrame", "SettingsPanel", "InterfaceOptionsFrame", "VideoOptionsFrame", "GameMenuFrame",
  "ColorPickerFrame", "EditModeManagerFrame", "AddonList", "HelpFrame", "MacroFrame", "KeyBindingFrame",
  "ContainerFrameCombinedBags", "BankFrame", "LootFrame", "MailFrame", "MerchantFrame", "TradeFrame",
  "WorldMapFrame", "CharacterFrame", "SpellBookFrame", "PlayerSpellsFrame", "FriendsFrame", "PVEFrame",
  "CollectionsJournal", "EncounterJournal", "AchievementFrame", "CommunitiesFrame", "GuildFrame",
  "QuestLogFrame", "QuestFrame", "GossipFrame", "AuctionHouseFrame", "AuctionFrame", "ProfessionsFrame",
  "TradeSkillFrame", "ClassTrainerFrame", "TalentFrame", "PlayerTalentFrame", "ChannelFrame",
  "CalendarFrame", "LFGParentFrame", "LFDParentFrame", "RaidFrame", "PetStableFrame", "TaxiFrame",
}
for i = 1, 13 do OCC_NAMES[#OCC_NAMES + 1] = "ContainerFrame" .. i end

local occNames, occAt, occSig, occSentAt = nil, -10, "", -10
-- Menus récents vus pendant leur ouverture (le gestionnaire ne donne que le premier niveau ; les
-- sous-menus sont repérés sous la souris, et parmi les cadres de la même strate que le menu).
local menuSeen = {}
-- Sous-menus ouverts à côté du menu (0.16.1 : sous-menu « Taille de la police » des réglages du
-- chat recouvert par le miroir tant que la souris restait sur le menu) : cadres visibles de la même
-- strate que le menu, rattachés à l'interface ou à un menu ouvert. Relevés 4 fois par seconde au
-- plus, seulement pendant qu'un menu est ouvert.
local menuScanAt, menuExtra = -10, {}
local MENU_SCAN = 0.25

local function MenuFamily(root, now)
  if now - menuScanAt < MENU_SCAN or not EnumerateFrames then return menuExtra end
  menuScanAt, menuExtra = now, {}
  local okS, strata = pcall(root.GetFrameStrata, root)
  if not okS or type(strata) ~= "string" then return menuExtra end
  local f, n = EnumerateFrames(), 0
  while f and n < 30000 do
    n = n + 1
    if f ~= root and not (f.IsForbidden and f:IsForbidden()) then
      local ok, vis, st, par = pcall(function() return f:IsVisible(), f:GetFrameStrata(), f:GetParent() end)
      if ok and vis and st == strata and (par == nil or par == UIParent or par == root or menuSeen[par]) then
        menuExtra[#menuExtra + 1] = f
      end
    end
    f = EnumerateFrames(f)
  end
  return menuExtra
end

local function OccNames()
  if occNames then return occNames end
  local seen, list = {}, {}
  for _, n in ipairs(OCC_NAMES) do
    if not seen[n] then seen[n] = true; list[#list + 1] = n end
  end
  -- Fenêtres de l'interface que le jeu range lui-même (personnage, carte, sorts...).
  if type(UIPanelWindows) == "table" then
    for n in pairs(UIPanelWindows) do
      if type(n) == "string" and not seen[n] then seen[n] = true; list[#list + 1] = n end
    end
  end
  occNames = list
  return list
end

-- Ombre portée d'un menu ou d'une bulle (texture ou atlas « shadow ») : ignorée.
local function Shadowy(r)
  local ok, t = pcall(function()
    return (r.GetAtlas and r:GetAtlas()) or (r.GetTexture and r:GetTexture())
  end)
  return ok and type(t) == "string" and t:lower():find("shadow", 1, true) ~= nil
end

-- Bord visible d'une boîte (0.16.1) : son cadre, agrandi de sa bordure quand elle dépasse (pièces
-- du NineSlice, bordure, fond), sans son ombre. Le miroir s'efface au ras de la boîte (avant : une
-- marge fixe de 10 à 14 pixels laissait un vide sombre autour des menus).
local function Rect(f, k0)
  if not (f and f.IsVisible and f:IsVisible()) then return nil end
  local l, b, w, h = f:GetLeft(), f:GetBottom(), f:GetWidth(), f:GetHeight()
  if not (l and b and w and h) or w <= 0 or h <= 0 then return nil end
  local L, B, R, T = l, b, l + w, b + h
  local function grow(x)
    if type(x) ~= "table" or not (x.IsVisible and x:IsVisible()) or Shadowy(x) then return end
    local xl, xb, xw, xh = x:GetLeft(), x:GetBottom(), x:GetWidth(), x:GetHeight()
    if not (xl and xb and xw and xh) then return end
    -- Une bordure, pas une autre fenêtre : au plus 16 unités hors du cadre.
    if xl < l - 16 or xb < b - 16 or xl + xw > l + w + 16 or xb + xh > b + h + 16 then return end
    L, B, R, T = min(L, xl), min(B, xb), max(R, xl + xw), max(T, xb + xh)
  end
  for _, key in ipairs({ "NineSlice", "Border", "Backdrop" }) do
    local border = f[key]
    if type(border) == "table" then
      grow(border)
      if border.GetRegions then
        for _, r in ipairs({ border:GetRegions() }) do grow(r) end
      end
    end
  end
  local k = f:GetEffectiveScale() * k0
  -- Un pixel de plus de chaque côté : le bord adouci de la bordure.
  return { floor(L * k) - 1, floor(B * k) - 1, ceil((R - L) * k) + 2, ceil((T - B) * k) + 2 }
end

local function Overlaps(r)
  for _, c in pairs(M.chatRects) do
    if r[1] < c[1] + c[3] and c[1] < r[1] + r[3] and r[2] < c[2] + c[4] and c[2] < r[2] + r[4] then return true end
  end
  return false
end

-- Rectangles des fenêtres du jeu affichées (carte, sacs, menus...), où qu'elles soient : les bulles
-- de dialogue qu'elles recouvrent ne sont pas traduites (0.15). Relus au plus 4 fois par seconde.
local uiAt, uiRects = -10, {}
M.Rect = Rect

function M:UiRects(k0)
  local now = GetTime()
  if now - uiAt < 0.25 then return uiRects end
  uiAt, uiRects = now, {}
  for _, n in ipairs(OccNames()) do
    local f = _G[n]
    if type(f) == "table" then
      local ok, r = pcall(Rect, f, k0)
      if ok and r then uiRects[#uiRects + 1] = r end
    end
  end
  return uiRects
end

function M:Occluders(now)
  if now - occAt < OCC_POLL then return end
  occAt = now
  local rects = {}
  if next(M.chatRects) then
    local _, ph = GetPhysicalScreenSize()
    local k0 = (ph or 768) / 768
    local function add(f)
      local ok, r = pcall(Rect, f, k0)
      if ok and r and Overlaps(r) and #rects < 12 then rects[#rects + 1] = r end
    end
    for _, n in ipairs(OccNames()) do
      local f = _G[n]
      if type(f) == "table" then add(f) end
    end
    local okM, mgr = pcall(function() return Menu and Menu.GetManager and Menu.GetManager() end)
    local okO, root = false, nil
    if okM and mgr then
      okO, root = pcall(function() return mgr:IsAnyMenuOpen() and mgr:GetOpenMenu() end)
    end
    if okO and root then
      menuSeen[root] = true
      -- Sous-menu ouvert : cadre de plus haut niveau de ce qui est sous la souris.
      local okF, top = pcall(function()
        local foci = GetMouseFoci and GetMouseFoci()
        local f = (type(foci) == "table" and foci[1]) or (GetMouseFocus and GetMouseFocus())
        local last, n = nil, 0
        while f and n < 12 do
          last = f
          f = f:GetParent()
          n = n + 1
        end
        return last
      end)
      if okF and top and top ~= UIParent and top ~= WorldFrame then menuSeen[top] = true end
      for m in pairs(menuSeen) do add(m) end
      local okE, extra = pcall(MenuFamily, root, now)
      if okE then
        for _, m in ipairs(extra) do add(m) end
      end
    elseif next(menuSeen) or #menuExtra > 0 then
      menuSeen, menuExtra, menuScanAt = {}, {}, -10
    end
  end
  local parts = {}
  for _, r in ipairs(rects) do parts[#parts + 1] = table.concat(r, ",") end
  local sig = table.concat(parts, ";")
  if sig == occSig and not self.occReset and (sig == "" or now - occSentAt < 3) then return end
  occSig, occSentAt, self.occReset = sig, now, nil
  local p = {}
  local room = min(#rects, floor((CT.MaxPayload() - 1) / 8))
  U8(p, room)
  for j = 1, room do
    local r = rects[j]
    U16(p, max(0, r[1])); U16(p, max(0, r[2])); U16(p, r[3]); U16(p, r[4])
  end
  CT.Enqueue(CT.FRAME_OCCLUDE, p, #p)
end

local SAMPLE = {
  { "|cff40ff40[Guilde]|r [|cffc79c6eHans|r] : Guten Abend zusammen! Schöne Grüße aus Köln.", 0.25, 1, 0.25 },
  { "[Groupe] [|cfff58cbaÉlise|r] : Ça marche, on se retrouve à l'entrée du donjon ?", 0.67, 0.67, 1 },
  { "[Иван] говорит: Привет всем! Кто идёт в подземелье?", 1, 1, 1 },
  { "[Νίκος] chuchote : Καλησπέρα! Θέλεις να έρθεις μαζί μας;", 1, 0.5, 1 },
  { "[2. Commerce] [Ana] : Vând |cffa335ee|Hitem:19019::::::::60:::::|h[Thunderfury]|h|r ieftin, scrieți-mi (ș ț ş ţ)!",
    1, 0.75, 0.75 },
  { "[Dire] [Zofia] : Cześć! Żółć gęślą jaźń. Cible |TInterface\\TargetingFrame\\UI-RaidTargetingIcon_1:0|t marquée.",
    1, 1, 1 },
}

function M:Command(arg, quiet)
  arg = (arg or ""):lower()
  if arg == "on" or arg == "calage" or arg == "off" then
    self.mode = arg
    self:Reset()
    if arg ~= "off" and #ChatFrames(GetTime()) == 0 then
      self.mode = "off"
      CT.Print("Chat miroir indisponible : aucune fenêtre de chat Blizzard compatible (addon de chat, ou version du jeu non prise en charge).")
      return
    end
    self:Poll(GetTime())
    if quiet then return end
    CT.Print(({
      on = "Chat miroir activé : le texte officiel est caché, l'application le redessine. /chevaliers off pour revenir.",
      calage = "Calage : texte officiel visible, l'application dessine par-dessus en magenta.",
      off = "Chat miroir désactivé : texte officiel rétabli.",
    })[arg])
  elseif arg == "sync" then
    self:Reset()
    CT.Print("Chat miroir : tout est renvoyé à l'application.")
  elseif arg == "echantillon" or arg == "sample" then
    -- Lignes de joueurs (comme un « dire ») : redessinées par l'application.
    local id = type(ChatTypeInfo) == "table" and ChatTypeInfo.SAY and ChatTypeInfo.SAY.id
    for _, s in ipairs(SAMPLE) do DEFAULT_CHAT_FRAME:AddMessage(s[1], s[2], s[3], s[4], id, nil, nil, "CHAT_MSG_SAY") end
  else
    CT.Print(format("Chat miroir : %s - descriptions %d, lignes %d, caractères mesurés %d, retours au texte officiel %d, listes tronquées %d",
      self.mode, self.stats.views, self.stats.lines, self.stats.measured or 0, self.stats.official, self.stats.truncated))
    CT.Print("Retrait des lignes coupées : " .. tostring(self.stats.indent or "pas encore mesuré"))
    if self.stats.measureError then CT.Print("Mesure de la police : " .. self.stats.measureError) end
    CT.Print("/chevaliers miroir on | calage | off | sync | echantillon")
  end
end

local driver = CreateFrame("Frame")
local acc = 0
driver:SetScript("OnUpdate", function(_, elapsed)
  acc = acc + (elapsed or 0)
  if acc < POLL then return end
  acc = 0
  if M.mode ~= "off" or next(M.state) then
    M:Poll(GetTime())
    if M.mode ~= "off" then
      local ok, err = pcall(M.Occluders, M, GetTime())
      if not ok then M.stats.occError = tostring(err) end
    end
    -- Mode coupé : une dernière description par fenêtre a été envoyée, on s'arrête.
    if M.mode == "off" then
      local allIdle = true
      for _, st in pairs(M.state) do if st.active then allIdle = false end end
      if allIdle then M.state = {} end
    end
  end
end)
