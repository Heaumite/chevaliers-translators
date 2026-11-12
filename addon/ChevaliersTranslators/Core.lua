-- Chevaliers Translators - Core.lua
-- Traduction du chat de WoW par l'application Chevaliers Translators, sur le PC du joueur.
--   relais du chat reçu vers l'application (bandeau de pixels en bas à droite de l'écran) ;
--   Maj+Entrée dans la boîte de saisie : envoi traduit (collé puis envoyé par l'application) ;
--   bouton Trad à droite de la barre de saisie : langue de l'envoi traduit ;
--   chat miroir (Mirror.lua) : les lignes des joueurs sont redessinées traduites par l'application.
-- Commandes : /chevaliers (aide) ; outils de développement : /chevaliers dev.

local ADDON, CT = ...

-- Version des fichiers Lua (le .toc n'est relu qu'au démarrage du jeu : après une mise à jour et
-- un /reload, il donnerait encore l'ancienne). Vérifiée contre l'application par un test.
CT.VERSION = "0.14.8"

local floor = math.floor
local format = string.format

local QUEUE_MAX = 400

local queue, qhead, qtail = {}, 1, 0
local rollingSeq = 0
local hold = 0.05
local nextSwap, ticks = 0, 0
local test
local chatOn = false
local msgId = 0
local stats = { sent = 0, secret = 0, dropped = 0, chat = 0 }
local syms, scratch, testBuf = {}, {}, {}

local function Print(msg)
  DEFAULT_CHAT_FRAME:AddMessage("|cff66ccff[Chevaliers]|r " .. msg)
end

-- API protégées par détection, jamais par numéro de version ----------------

local function InLockdown()
  return C_ChatInfo and C_ChatInfo.InChatMessagingLockdown and C_ChatInfo.InChatMessagingLockdown() or false
end

local function Secret(v) return issecretvalue ~= nil and issecretvalue(v) or false end
-- Vrai seulement pour une valeur lisible et vraie. Une valeur secrète (WoW 12, instances de JcJ,
-- donjons, raids) ne doit être ni testée ni comparée avant ce contrôle : « attempt to perform
-- boolean test on a secret boolean value » (0.14.3, UnitIsAFK en champ de bataille).
local function Truthy(v) return not Secret(v) and v and true or false end
CT.Truthy = Truthy

-- Contexte du joueur pour l'application (0.13.3) : bit 1 en combat, 2 en instance, 4 en raid,
-- 8 absent (AFK), 16 chat verrouillé. Les bascules de modèle attendent la fin du combat.
local function ContextFlags()
  local f = 0
  local ok, c = pcall(function() return InCombatLockdown() or (UnitAffectingCombat and UnitAffectingCombat("player")) end)
  if ok and Truthy(c) then f = f + 1 end
  local okI, inInst, kind = pcall(IsInInstance)
  if okI and Truthy(inInst) then
    f = f + 2
    if not Secret(kind) and kind == "raid" then f = f + 4 end
  end
  local okA, afk = pcall(function() return UnitIsAFK and UnitIsAFK("player") end)
  if okA and Truthy(afk) then f = f + 8 end
  if InLockdown() then f = f + 16 end
  return f
end
CT.ContextFlags = ContextFlags

-- Réglages graphiques de WoW (0.14.2) : numéro changé à chaque réglage graphique modifié dans le
-- jeu (qualité, échelle de rendu, textures, résolution, mode d'affichage...), envoyé avec la trame
-- de vie ; l'application réévalue alors la place du modèle de traduction dans la mémoire vidéo.
local gfxGen = 0
local GFX_WORDS = { "graphics", "render", "texture", "resolution", "msaa", "shadow", "farclip", "lod",
  "gxmaximize", "gxwindow", "gxmonitor", "gxapi", "ssao", "liquid", "particle", "ffx", "vsync", "raidsettings",
  "density", "clutter", "environment", "projected", "dlss", "xess", "fsr", "antialias", "sharpen",
  "viewdistance", "outline", "weather", "depth", "computeeffects", "raytrac" }
local function IsGraphicsCVar(name)
  if type(name) ~= "string" or Secret(name) then return false end
  local n = name:lower()
  for _, w in ipairs(GFX_WORDS) do
    if n:find(w, 1, true) then return true end
  end
  return false
end
CT.IsGraphicsCVar = IsGraphicsCVar
local gfxWatch = CreateFrame("Frame")
gfxWatch:RegisterEvent("CVAR_UPDATE")
gfxWatch:RegisterEvent("DISPLAY_SIZE_CHANGED")
gfxWatch:SetScript("OnEvent", function(_, event, name)
  if event == "CVAR_UPDATE" and not IsGraphicsCVar(name) then return end
  gfxGen = gfxGen % 255 + 1
end)
function CT.GraphicsGen() return gfxGen end

-- En combat (0.14, demande du 11/11) : le miroir ne redessine rien, le chat officiel de WoW
-- s'affiche ; la traduction continue, et les lignes reviennent traduites une fois le combat fini.
-- Dispositif télévision (0.14) : l'application reconnaît l'écran de WoW avant la connexion et
-- le note dans sa présence (e = 1 : télévision). Le texte du chat du jeu n'est alors pas caché :
-- le chat traduit le recouvre ; si rien ne peut s'afficher par-dessus le jeu sur cet écran, le
-- joueur garde le chat du jeu. Réglage : /chevaliers tele auto | on | off.
function CT.CoverMode()
  local db = ChevaliersTranslatorsDB
  local want = db and db.tele or "auto"
  if want == "on" then return true end
  if want == "off" then return false end
  return CT.screenTV and true or false
end

function CT.InCombat()
  local ok, c = pcall(function() return InCombatLockdown() or (UnitAffectingCombat and UnitAffectingCombat("player")) end)
  return ok and Truthy(c)
end

local function IsLoadedCompat(name)
  if C_AddOns and C_AddOns.IsAddOnLoaded then return C_AddOns.IsAddOnLoaded(name) end
  if IsAddOnLoaded then return IsAddOnLoaded(name) end
  return false
end

local function NumAddOns()
  if C_AddOns and C_AddOns.GetNumAddOns then return C_AddOns.GetNumAddOns() end
  if GetNumAddOns then return GetNumAddOns() end
  return 0
end

local function AddOnName(i)
  if C_AddOns and C_AddOns.GetAddOnInfo then return (C_AddOns.GetAddOnInfo(i)) end
  if GetAddOnInfo then return (GetAddOnInfo(i)) end
end

local function SafeCall(fn, ...)
  if type(fn) ~= "function" then return nil end
  local ok, a, b = pcall(fn, ...)
  if ok then return a, b end
end

local function SafeNumLanguages()
  if GetNumLanguages and GetLanguageByIndex then return GetNumLanguages() end
  return 0
end

-- Nom d'une langue à partir de son identifiant (le champ `language` de la boîte peut être périmé).
local function LanguageName(id)
  if type(id) ~= "number" then return nil end
  local n = SafeNumLanguages()
  for i = 1, n do
    local name, langId = GetLanguageByIndex(i)
    if langId == id then return name end
  end
  return nil
end

-- Langue choisie dans la boîte de chat, sinon langue par défaut de la faction.
-- C'est celle que le jeu utilise pour tes propres messages (champ languageID de la boîte).
local function ChatLanguage(editBox)
  local eb = editBox or (ChatEdit_GetActiveWindow and ChatEdit_GetActiveWindow())
    or (DEFAULT_CHAT_FRAME and DEFAULT_CHAT_FRAME.editBox)
  local id = eb and eb.languageID
  if type(id) == "number" then return id, LanguageName(id) or tostring(id), "boîte de chat" end
  if GetDefaultLanguage then
    local defName, defId = GetDefaultLanguage()
    if type(defId) == "number" then return defId, tostring(defName), "langue par défaut" end
  end
  return nil, "non précisée", "aucune"
end

-- Forever : les personnages ont un prénom et un nom (« Elzhem Batonnoir ») et UnitName
-- peut ne renvoyer que le prénom. Le nom complet est appris depuis nos propres messages.
local selfName

local function Now()
  return debugprofilestop and debugprofilestop() or (GetTime() * 1000)
end

-- File de trames ------------------------------------------------------------

local function MaxPayload()
  return select(3, CT.Capacity(CT.Banner.cols))
end

-- Numéro de séquence donné à l'envoi (0.12.3) : l'application compte les trames manquées
-- d'après les trous dans la suite, quel que soit l'ordre des files.
local function Enqueue(ftype, payload, len)
  if qtail - qhead + 1 >= QUEUE_MAX then
    queue[qhead] = nil
    qhead = qhead + 1
    stats.dropped = stats.dropped + 1
  end
  qtail = qtail + 1
  queue[qtail] = { t = ftype, p = payload, n = len }
end

-- File prioritaire : envoi traduit (Maj+Entrée) et brouillons passent devant le chat miroir.
local prio, phead, ptail = {}, 1, 0

local function EnqueuePrio(ftype, payload, len)
  ptail = ptail + 1
  prio[ptail] = { t = ftype, p = payload, n = len }
end

-- File de fond (0.12.3) : messages qu'aucune fenêtre de chat visible n'affiche (onglet Services ou
-- Commerce en arrière-plan). Ils passent quand le reste est parti, et sont abandonnés au-delà de
-- LOW_TTL secondes : le chat affiché n'attend plus derrière le spam des autres onglets. 0.12.4 :
-- 60 s (tout message étranger est traduit, même celui d'un onglet caché).
local LOW_MAX, LOW_TTL = 400, 60
local low, lhead, ltail = {}, 1, 0
stats.lowDropped = 0

local function DropLowHead()
  local tag = low[lhead].tag
  while lhead <= ltail and low[lhead].tag == tag do
    low[lhead] = nil
    lhead = lhead + 1
  end
  stats.lowDropped = stats.lowDropped + 1
end

local function EnqueueLow(ftype, payload, len, tag)
  while ltail - lhead + 1 >= LOW_MAX do DropLowHead() end
  ltail = ltail + 1
  low[ltail] = { t = ftype, p = payload, n = len, tag = tag, at = GetTime() }
end

-- Partagés avec Mirror.lua.
CT.Print, CT.InLockdown, CT.MaxPayload, CT.Enqueue = Print, InLockdown, MaxPayload, Enqueue

local aliveBuf, waitBuf = { 1, 0, 0 }, { 0 }
-- Bandeau immobile quand il n'y a rien à envoyer (0.13.3) : la dernière trame de vie reste à
-- l'écran au lieu de clignoter ; l'application la voit toujours, c'est son signe de vie.
-- -1 : rien d'immobile ; -2 : signal d'attente ; sinon les drapeaux de la trame de vie affichée.
local idleShown = -1

local function Stamp(item)
  rollingSeq = (rollingSeq + 1) % 65536
  item.seq = rollingSeq
  return item
end

-- Trames en attente (toutes files) : diagnostic de la latence du chat.
local function Backlog()
  return (ptail - phead + 1) + (qtail - qhead + 1) + (ltail - lhead + 1)
end
CT.Backlog = Backlog

local function NextItem()
  if phead <= ptail then
    local item = prio[phead]
    prio[phead] = nil
    phead = phead + 1
    return Stamp(item)
  end
  if qhead <= qtail then
    local item = queue[qhead]
    queue[qhead] = nil
    qhead = qhead + 1
    return Stamp(item)
  end
  -- File de fond : un message trop ancien est abandonné en entier (tous ses fragments).
  while lhead <= ltail and GetTime() - low[lhead].at > LOW_TTL do DropLowHead() end
  if lhead <= ltail then
    local item = low[lhead]
    low[lhead] = nil
    lhead = lhead + 1
    return Stamp(item)
  end
  -- Plus rien à envoyer : trame de vie immobile (voir IdleItem).
  if test and test.next < test.total then
    local s = test.next
    test.next = s + 1
    local n = CT.TestPayload(s, MaxPayload(), testBuf)
    if test.next >= test.total then
      Print(format("Test terminé : %d trames émises en %.1f s.", test.total, (Now() - test.t0) / 1000))
      test = nil
    end
    return { t = CT.FRAME_TEST, seq = s, p = testBuf, n = n }
  end
end

-- File du bandeau qui ne se vide plus (chat très actif) : une ligne dans la console de
-- l'application, au plus toutes les 30 s (0.12.3).
local backlogAt, backlogDropped = -100, 0
local function WatchBacklog(now)
  if now - backlogAt < 30 then return end
  local n = Backlog()
  local lost = stats.dropped + stats.lowDropped - backlogDropped
  if n * hold < 3 and lost == 0 then return end
  backlogAt, backlogDropped = now, stats.dropped + stats.lowDropped
  local text = format("bandeau : %d trames en attente (%.1f s), %d messages abandonnés depuis 30 s", n, n * hold, lost)
  local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
  EnqueuePrio(CT.FRAME_NOTE, p, #p)
end

-- Rien à envoyer : la trame de vie (miroir actif) ou le signal d'attente (application absente à
-- la connexion), peints une fois et laissés à l'écran ; repeints seulement quand le contexte
-- change. Sinon le bandeau est caché. Renvoie la trame à peindre, ou nil (rien à changer).
local idleCheck = 0
local function IdleItem()
  if chatOn and CT.Mirror and CT.Mirror.mode and CT.Mirror.mode ~= "off" then
    if idleShown >= 0 and GetTime() - idleCheck < 0.25 then return nil end
    idleCheck = GetTime()
    local f = ContextFlags()
    -- Repeinte quand le contexte ou les réglages graphiques changent.
    local key = f + 256 * gfxGen
    if idleShown == key then return nil end
    idleShown = key
    aliveBuf[2] = f
    aliveBuf[3] = gfxGen
    rollingSeq = (rollingSeq + 1) % 65536
    return { t = CT.FRAME_ALIVE, seq = rollingSeq, p = aliveBuf, n = 3 }
  end
  if not chatOn and CT.waiting then
    if idleShown == -2 then return nil end
    idleShown = -2
    rollingSeq = (rollingSeq + 1) % 65536
    return { t = CT.FRAME_ALIVE, seq = rollingSeq, p = waitBuf, n = 1 }
  end
  idleShown = -1
  CT.Banner:Hide()
  return nil
end

local driver = CreateFrame("Frame")
driver:SetScript("OnUpdate", function()
  ticks = ticks + 1
  local now = GetTime()
  if chatOn then WatchBacklog(now) end
  -- Cadence planifiée : l'attente n'est pas arrondie à l'image suivante, la moyenne
  -- tient le nombre de trames demandé. Chaque trame reste au moins 2 images à l'écran.
  if now < nextSwap or ticks < 2 then return end
  if CT.Banner.cols == 0 then return end
  local item = NextItem()
  if item then
    idleShown = -1
  else
    item = IdleItem()
    if not item then return end
  end
  local ok, err = CT.BuildBanner(item.t, item.seq, item.p, item.n, CT.Banner.cols, syms, scratch)
  if ok then
    CT.Banner:Paint(syms)
    stats.sent = stats.sent + 1
  else
    Print("Trame ignorée : " .. tostring(err))
  end
  if now - nextSwap > hold then
    nextSwap = now + hold -- reprise après une pause ou une chute d'images
  else
    nextSwap = nextSwap + hold
  end
  ticks = 0
end)

-- S2 : relais du chat ---------------------------------------------------------

-- Tous les messages écrits par des joueurs (0.12) ; ceux du jeu (système, butin, PNJ, émotes
-- standard) sont déjà dans la langue du client.
local CHAT_EVENTS = {
  CHAT_MSG_SAY = "SAY", CHAT_MSG_YELL = "YELL", CHAT_MSG_EMOTE = "EMOTE",
  CHAT_MSG_WHISPER = "WHISPER", CHAT_MSG_WHISPER_INFORM = "WHISPER_INFORM",
  CHAT_MSG_BN_WHISPER = "BN_WHISPER", CHAT_MSG_BN_WHISPER_INFORM = "BN_WHISPER_INFORM",
  CHAT_MSG_PARTY = "PARTY", CHAT_MSG_PARTY_LEADER = "PARTY_LEADER",
  CHAT_MSG_RAID = "RAID", CHAT_MSG_RAID_LEADER = "RAID_LEADER", CHAT_MSG_RAID_WARNING = "RAID_WARNING",
  CHAT_MSG_GUILD = "GUILD", CHAT_MSG_OFFICER = "OFFICER", CHAT_MSG_GUILD_DISCORD = "GUILD_DISCORD",
  CHAT_MSG_INSTANCE_CHAT = "INSTANCE_CHAT", CHAT_MSG_INSTANCE_CHAT_LEADER = "INSTANCE_CHAT_LEADER",
  CHAT_MSG_CHANNEL = "CHANNEL", CHAT_MSG_COMMUNITIES_CHANNEL = "COMMUNITIES_CHANNEL",
  CHAT_MSG_VOICE_TEXT = "VOICE_TEXT", CHAT_MSG_AFK = "AFK", CHAT_MSG_DND = "DND",
  CHAT_MSG_BN_INLINE_TOAST_BROADCAST = "BN_BROADCAST",
}
-- Partagé avec Mirror.lua : une ligne du chat issue de ces événements est redessinée (traduite).
CT.CHAT_EVENTS = CHAT_EVENTS
-- Messages envoyés par le joueur (l'expéditeur indiqué est le destinataire).
local INFORM = { CHAT_MSG_WHISPER_INFORM = true, CHAT_MSG_BN_WHISPER_INFORM = true }

-- Messages reçus par type : total et secrets (diagnostic).
local eventCounts = {}

-- Langue raciale que le personnage ne parle pas (« [Orc] revas il rath'is ») : du charabia
-- produit par le jeu, rien à traduire.
local function UnknownLanguage(langName)
  if type(langName) ~= "string" or langName == "" then return false end
  if issecretvalue and issecretvalue(langName) then return false end
  for i = 1, SafeNumLanguages() do
    if GetLanguageByIndex(i) == langName then return false end
  end
  return true
end

-- Le message sera-t-il affiché par une fenêtre de chat visible ? Même règle que les fenêtres de
-- Blizzard : événement enregistré par la fenêtre ; pour les canaux numérotés, nom du canal dans sa
-- liste (ou numéro de canal de zone). Dans le doute (fonction absente, erreur) : oui.
local regCheck, regOk = -10, true
local function Displayed(event, chanBase, zoneId)
  if not (CT.Mirror and CT.Mirror.Frames) then return true end
  local ok, res = pcall(function()
    local frames = CT.Mirror:Frames()
    local now = GetTime()
    if now - regCheck > 5 then
      -- Les fenêtres du jeu enregistrent-elles encore leurs événements ? Sinon la règle ne tient pas.
      regCheck, regOk = now, false
      for _, e in ipairs(frames) do
        if e.cf:IsEventRegistered("CHAT_MSG_SAY") or e.cf:IsEventRegistered("CHAT_MSG_CHANNEL")
          or e.cf:IsEventRegistered("CHAT_MSG_GUILD") or e.cf:IsEventRegistered("CHAT_MSG_WHISPER") then
          regOk = true
          break
        end
      end
    end
    if not regOk then return true end
    local any = false
    for _, e in ipairs(frames) do
      local cf = e.cf
      if cf:IsVisible() then
        any = true
        if cf:IsEventRegistered(event) then
          if event ~= "CHAT_MSG_CHANNEL" then return true end
          local list = cf.channelList
          if type(list) ~= "table" then return true end
          local base = type(chanBase) == "string" and chanBase:upper() or nil
          for idx, name in pairs(list) do
            if base and type(name) == "string" and name:upper() == base then return true end
            if type(zoneId) == "number" and zoneId > 0 and type(cf.zoneChannelList) == "table"
              and cf.zoneChannelList[idx] == zoneId then return true end
          end
        end
      end
    end
    return not any
  end)
  return not ok or res
end

local chatFrame = CreateFrame("Frame")
for event in pairs(CHAT_EVENTS) do
  pcall(chatFrame.RegisterEvent, chatFrame, event)
end
-- Messages reçus pendant le verrouillage du chat (combat en instance, 0.13.3) : texte et
-- expéditeur sont secrets, l'addon ne peut ni les lire ni les relayer. Ils sont gardés tels quels
-- (sans les lire) et relayés à la fin du verrouillage s'ils sont devenus lisibles ; l'application
-- affiche alors leurs traductions d'un bloc. Pendant le combat, le chat officiel s'affiche.
-- Zone et canaux géographiques (0.13.3) : Trad:AUTO prend la langue du canal dans la zone
-- actuelle (le canal Général d'une zone n'a pas les mêmes joueurs que celui d'une autre), et
-- celle des joueurs proches pour Dire et Crier.
local zoneGeo, zoneMap, zoneSent = {}, 0, ""
local function SendZone()
  if not chatOn then return end
  local ids = {}
  for idx in pairs(zoneGeo) do ids[#ids + 1] = idx end
  table.sort(ids)
  local key = zoneMap .. ":" .. table.concat(ids, ",")
  if key == zoneSent then return end
  zoneSent = key
  local p = { floor(zoneMap / 256) % 256, zoneMap % 256 }
  for _, i in ipairs(ids) do p[#p + 1] = i end
  EnqueuePrio(CT.FRAME_ZONE, p, #p)
end
CT.SendZone = function() zoneSent = ""; SendZone() end

local zoneWatch = CreateFrame("Frame")
pcall(zoneWatch.RegisterEvent, zoneWatch, "ZONE_CHANGED_NEW_AREA")
pcall(zoneWatch.RegisterEvent, zoneWatch, "PLAYER_ENTERING_WORLD")
zoneWatch:SetScript("OnEvent", function()
  local ok, m = pcall(function() return C_Map and C_Map.GetBestMapForUnit and C_Map.GetBestMapForUnit("player") end)
  zoneMap = (ok and type(m) == "number" and not Secret(m)) and m % 65536 or 0
  SendZone()
end)

local held, heldMax = {}, 300
local heldStats = { held = 0, late = 0, secret = 0 }
local Relay

local function Hold(event, text, sender, ...)
  if #held >= heldMax then table.remove(held, 1) end
  held[#held + 1] = { event = event, text = text, sender = sender, n = select("#", ...), ... }
  heldStats.held = heldStats.held + 1
end

local function ReleaseHeld()
  if #held == 0 or InLockdown() then return end
  local list = held
  held = {}
  local late, still = 0, 0
  for _, h in ipairs(list) do
    if Secret(h.text) or Secret(h.sender) then
      still = still + 1
    elseif Relay(h.event, true, h.text, h.sender, unpack(h, 1, h.n)) then
      late = late + 1
    end
  end
  heldStats.late, heldStats.secret = heldStats.late + late, heldStats.secret + still
  local text = format("combat : %d messages reçus pendant le verrouillage du chat, %d relayés après, %d restés secrets", #list, late, still)
  local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
  EnqueuePrio(CT.FRAME_NOTE, p, #p)
end
CT.HeldStats = heldStats

local heldWatch = CreateFrame("Frame")
local heldAcc = 0
heldWatch:SetScript("OnUpdate", function(_, elapsed)
  heldAcc = heldAcc + (elapsed or 0)
  if heldAcc < 1 then return end
  heldAcc = 0
  ReleaseHeld()
end)

chatFrame:SetScript("OnEvent", function(_, event, text, sender, ...)
  local c = eventCounts[event] or { n = 0, secret = 0 }
  eventCounts[event] = c
  c.n = c.n + 1
  -- Retail et Forever : sous verrouillage, texte et expéditeur sont secrets. On ne les touche
  -- pas (pas même leur longueur) : gardés pour la fin du verrouillage, la ligne reste en texte
  -- officiel.
  if Secret(text) or Secret(sender) then
    c.secret = c.secret + 1
    if chatOn then
      stats.secret = stats.secret + 1
      Hold(event, text, sender, ...)
    end
    return
  end
  Relay(event, false, text, sender, ...)
end)

-- Relais d'un message lisible ; `late` : reçu pendant le verrouillage, relayé après.
Relay = function(event, late, text, sender, ...)
  if type(text) ~= "string" or type(sender) ~= "string" then return false end
  -- 12e argument de CHAT_MSG_* : identifiant de l'expéditeur (du destinataire pour WHISPER_INFORM).
  local guid = select(10, ...)
  local own = type(guid) == "string" and not (issecretvalue and issecretvalue(guid)) and guid == UnitGUID("player")
  if own then
    selfName = sender
  end
  if not chatOn then return false end
  if not own and UnknownLanguage((...)) then return false end
  msgId = (msgId + 1) % 65536
  stats.chat = stats.chat + 1
  -- Bit de poids fort du canal : message du joueur lui-même.
  local code = CT.CHANNEL_CODES[CHAT_EVENTS[event]] or 0
  if event == "CHAT_MSG_CHANNEL" then
    -- 8e argument : numéro du canal (1. Général, 2. Commerce...).
    local idx = select(6, ...)
    if not (issecretvalue and issecretvalue(idx)) then
      idx = tonumber(idx)
      if idx and idx >= 1 and idx <= 63 then
        code = CT.CHANNEL_NUMBERED + idx
        -- 7e argument : canal de zone (Général, Commerce, Défense locale...) s'il n'est pas nul.
        local zid = select(5, ...)
        if not Secret(zid) and (tonumber(zid) or 0) ~= 0 and not zoneGeo[idx] then
          zoneGeo[idx] = true
          SendZone()
        end
      end
    end
  end
  if own or INFORM[event] then code = code + 128 end
  -- Message qu'aucune fenêtre visible n'affiche : file de fond.
  local shown = own or INFORM[event] or Displayed(event, select(7, ...), select(5, ...))
  if late then
    -- Reçu pendant le combat : l'application le traduit et l'affiche avec les autres du combat.
    if own then return false end
    CT.ChatFragments(msgId, code, sender, text, MaxPayload(),
      function(p, len) Enqueue(CT.FRAME_LATE, p, len) end)
    if CT.Mirror and CT.Mirror.NoteChat then CT.Mirror:NoteChat(msgId, text) end
    return true
  end
  if shown then
    CT.ChatFragments(msgId, code, sender, text, MaxPayload(),
      function(p, len) Enqueue(CT.FRAME_CHAT, p, len) end)
    -- Le miroir cite ce message dans sa ligne par son numéro au lieu de le renvoyer (0.12.3).
    if CT.Mirror and CT.Mirror.NoteChat then CT.Mirror:NoteChat(msgId, text) end
  else
    local tag = msgId
    CT.ChatFragments(msgId, code, sender, text, MaxPayload(),
      function(p, len) EnqueueLow(CT.FRAME_CHAT, p, len, tag) end)
  end
  return true
end

-- S3 : Maj+Entrée -------------------------------------------------------------
-- Maj+Entrée dans la boîte de saisie : le texte part au traducteur, la boîte se rouvre vide sur
-- le même canal et pour le même destinataire, et l'addon signale qu'elle est prête. L'application
-- y colle la traduction (Ctrl+V) puis l'envoie (Entrée) : deux touches simulées, avec l'accord du
-- joueur (09/11/2026), seulement à ce moment-là et seulement quand la boîte est prête. L'envoi
-- est donc celui du joueur : tous les canaux, Dire et canaux numérotés compris.
-- Taper un caractère pendant l'attente annule ; sans traduction en 6 s, le texte d'origine est
-- remis dans la boîte. Pendant la frappe, le brouillon part d'avance à la traduction : au moment
-- de l'appui, elle est en général prête. Aucun emplacement n'est consommé.
-- Retail et Forever : interception uniquement par le rappel Blizzard prévu pour les addons ;
-- aucun script de la boîte de saisie n'est remplacé (crochets en lecture seulement).

-- Langue de l'envoi traduit : AUTO (devinée par l'application : langue du destinataire d'un
-- chuchotement, sinon langue majoritaire des derniers messages du canal) ou une langue choisie
-- (menu du bouton Trad, ou /chevaliers langue xx). Après un choix à la main et un envoi, retour à AUTO
-- au bout de 60 s.
local dest = "AUTO"
local revertAt          -- retour à AUTO prévu
local awaiting          -- envoi en attente du collage
local lastDraft         -- dernier brouillon envoyé
local draftDue, draftBox
local reqCounter = 0
local s3Method = "non installée"
local s3Stats = { requests = 0, sent = 0, cancelled = 0, timeouts = 0, drafts = 0 }
local REVERT_DELAY = 60
local DRAFT_DELAY = 0.6
local PASTE_TIMEOUT = 6
local STATE_READY, STATE_CANCEL, STATE_SENT = 1, 2, 3
local tradButtons = {}

local function SetDest(code)
  dest = code
  revertAt = nil
  if ChevaliersTranslatorsDB then ChevaliersTranslatorsDB.dest = code end
  for _, b in ipairs(tradButtons) do b:SetText("Trad:" .. dest) end
end

local function NextReq()
  reqCounter = (reqCounter % 65535) + 1
  return reqCounter
end

local function BoxChatType(eb)
  return (eb.GetChatType and eb:GetChatType()) or eb:GetAttribute("chatType") or "SAY"
end

local function Target(editBox, chatType)
  local t
  if chatType == "WHISPER" or chatType == "BN_WHISPER" then
    t = (editBox.GetTellTarget and editBox:GetTellTarget()) or editBox:GetAttribute("tellTarget")
  elseif chatType == "CHANNEL" then
    t = editBox:GetAttribute("channelTarget")
  end
  return t and tostring(t) or nil
end

-- Demande à l'application : « s|b\31langue\31cible\31canal\31destinataire » + texte ;
-- « s » : envoi (Maj+Entrée), « b » : brouillon pendant la frappe.
local function SendRequest(id, kind, langId, chatType, target, text)
  local meta = format("%s\031%d\031%s\031%s\031%s", kind, langId or 0, dest, chatType, target or "")
  CT.ChatFragments(id, 0, meta, text, MaxPayload(), function(p, len) EnqueuePrio(CT.FRAME_SEND, p, len) end)
end

-- Liens d'objets, de sorts, de quêtes d'un message, dans l'ordre (même découpe que l'application).
local function Links(text)
  local out, from = {}, 1
  while true do
    local i = text:find("|H", from, true)
    if not i then break end
    local a = text:find("|h", i + 2, true)
    if not a then break end
    local b = text:find("|h", a + 2, true)
    if not b then break end
    local start = i
    if i > 10 and text:sub(i - 10, i - 9) == "|c" and text:sub(i - 8, i - 1):match("^%x+$") then start = i - 10 end
    local stop = b + 1
    if text:sub(stop + 1, stop + 2) == "|r" then stop = stop + 2 end
    out[#out + 1] = text:sub(start, stop)
    from = stop + 1
  end
  return out
end
CT.Links = Links

-- Deux fois : une trame ne reste que quelques images à l'écran.
local function SendState(kind, id)
  local p = { kind, floor(id / 256) % 256, id % 256 }
  EnqueuePrio(CT.FRAME_STATE, p, 3)
  EnqueuePrio(CT.FRAME_STATE, p, 3)
end

-- Rouvre la boîte de saisie telle qu'elle était : canal, destinataire, langue du personnage.
local function Reopen(s)
  local eb = s.box
  if ChatFrameUtil and ChatFrameUtil.ActivateChat then
    ChatFrameUtil.ActivateChat(eb)
  elseif ChatEdit_ActivateChat then
    ChatEdit_ActivateChat(eb)
  else
    if eb.Show then eb:Show() end
    if eb.SetFocus then eb:SetFocus() end
  end
  -- Seulement ce que le jeu a remis à zéro en fermant la boîte (Crier, ou un chuchotement quand il
  -- n'est pas « collant ») : le moins d'écritures possible dans la boîte de Blizzard.
  local changed = false
  if BoxChatType(eb) ~= s.chatType then
    if eb.SetChatType then eb:SetChatType(s.chatType) else eb:SetAttribute("chatType", s.chatType) end
    changed = true
  end
  if (s.chatType == "WHISPER" or s.chatType == "BN_WHISPER") and s.target and Target(eb, s.chatType) ~= s.target then
    if eb.SetTellTarget then eb:SetTellTarget(s.target) else eb:SetAttribute("tellTarget", s.target) end
    changed = true
  elseif s.chatType == "CHANNEL" and s.target and Target(eb, "CHANNEL") ~= s.target then
    if eb.SetChannelTarget then eb:SetChannelTarget(s.target) else eb:SetAttribute("channelTarget", s.target) end
    changed = true
  end
  if changed then
    if eb.UpdateHeader then eb:UpdateHeader() elseif ChatEdit_UpdateHeader then ChatEdit_UpdateHeader(eb) end
  end
  s.reopened = true
  if s.restore then
    -- Application absente : le texte revient tel quel.
    eb:SetText(s.text)
    awaiting = nil
    Print("Traduction inactive : l'application n'est pas lancée (ou /chevaliers off). Ton texte est remis dans la boîte.")
    return
  end
  if eb:GetText() ~= "" then eb:SetText("") end
  s.len = 0
  SendState(STATE_READY, s.id)
end

local function Cancel(why)
  local s = awaiting
  if not s then return end
  awaiting = nil
  s3Stats.cancelled = s3Stats.cancelled + 1
  SendState(STATE_CANCEL, s.id)
  if why then Print(why) end
end

-- Brouillon : après une courte pause dans la frappe, le texte part à la traduction ; l'application
-- garde le résultat pour l'appui sur Maj+Entrée.
local function SendDraft(editBox)
  if not chatOn then return end
  if awaiting and awaiting.box == editBox then return end
  local text = editBox:GetText()
  if issecretvalue and issecretvalue(text) then return end
  if type(text) ~= "string" or not text:find("%a") or text:sub(1, 1) == "/" then return end
  local chatType = BoxChatType(editBox)
  local target = Target(editBox, chatType)
  local d = lastDraft
  if d and d.text == text and d.dest == dest and d.chatType == chatType and d.target == target then return end
  lastDraft = { text = text, dest = dest, chatType = chatType, target = target }
  local langId = editBox.languageID
  s3Stats.drafts = s3Stats.drafts + 1
  SendRequest(NextReq(), "b", type(langId) == "number" and langId or nil, chatType, target, text)
end

-- Démarre un envoi traduit pour le texte courant de la boîte : la boîte est vidée (le jeu
-- n'envoie rien tant que la traduction n'est pas prête), puis rouverte vide ; l'application y
-- colle la traduction et l'envoie. Partagé par Maj+Entrée et Maj+clic sur le bouton Trad (0.14.7).
-- Faux si rien à envoyer (boîte vide, commande « / », texte secret) : le jeu suit son cours.
local function StartSend(editBox)
  if not editBox or not editBox.GetText then return false end
  local text = editBox:GetText()
  if issecretvalue and issecretvalue(text) then return false end
  if type(text) ~= "string" or not text:find("%S") or text:sub(1, 1) == "/" then return false end
  local chatType = BoxChatType(editBox)
  -- Texte vidé d'abord : une erreur plus bas ne laisse jamais partir le texte non traduit.
  editBox:SetText("")
  draftDue = nil
  if awaiting then Cancel() end
  local langId = editBox.languageID
  local s = { id = NextReq(), text = text, box = editBox, chatType = chatType, target = Target(editBox, chatType),
    lang = type(langId) == "number" and langId or nil, at = GetTime(), reopenAt = GetTime() }
  awaiting = s
  if not chatOn then
    s.restore = true
    return true
  end
  s3Stats.requests = s3Stats.requests + 1
  SendRequest(s.id, "s", s.lang, chatType, s.target, text)
  return true
end
CT.StartSend = StartSend

-- Appelé par le jeu juste avant qu'il lise le texte à envoyer.
local function OnPreSend(editBox)
  local text = editBox:GetText()
  if issecretvalue and issecretvalue(text) then return end
  if type(text) ~= "string" then return end
  -- La traduction collée part (Entrée simulée, ou Entrée du joueur) : rien à intercepter.
  local s = awaiting
  if s and s.reopened and s.box == editBox then
    if text:find("%S") then
      -- Liens d'objets : collés sous forme de repères {1}, {2}... (le jeu protège les « | »
      -- collés) ; les vrais liens, ceux du texte d'origine, sont remis ici juste avant l'envoi.
      if text:find("{%d+}") then
        local links = Links(s.text)
        local out = text:gsub("{(%d+)}", function(n) return links[tonumber(n)] end)
        if out ~= text then editBox:SetText(out) end
      end
      awaiting = nil
      s3Stats.sent = s3Stats.sent + 1
      SendState(STATE_SENT, s.id)
      if dest ~= "AUTO" then revertAt = GetTime() + REVERT_DELAY end
    end
    return
  end
  if not IsShiftKeyDown() then return end
  StartSend(editBox)
end

local function ULen(text)
  if strlenutf8 then return strlenutf8(text) end
  local n = 0
  for _ in text:gmatch("[%z\1-\127\194-\244][\128-\191]*") do n = n + 1 end
  return n
end

-- Pendant l'attente du collage : le collage fait apparaître toute la traduction d'un coup ; un
-- caractère de plus, c'est le joueur qui écrit autre chose : on annule, sans message (essai du
-- 09/11/2026 : le jeu signale aussi le collage comme une frappe, OnChar ne les distingue pas).
local function WatchPaste(editBox)
  local s = awaiting
  if not (s and s.reopened and s.box == editBox) or s.pasted then return end
  local text = editBox:GetText()
  if type(text) ~= "string" or (issecretvalue and issecretvalue(text)) then return end
  local n = ULen(text)
  if n - (s.len or 0) >= 2 then
    s.pasted = true
  elseif n - (s.len or 0) == 1 then
    Cancel()
  end
  s.len = n
end

local function OnDraftChanged(editBox, userInput)
  local ok, err = pcall(WatchPaste, editBox)
  if not ok then Print("|cffff4040Erreur S3 : " .. tostring(err) .. "|r") end
  if userInput and chatOn and not (awaiting and awaiting.box == editBox) then
    draftDue = GetTime() + DRAFT_DELAY
    draftBox = editBox
  end
end

local function OnBoxClosed(editBox)
  local s = awaiting
  if s and s.reopened and s.box == editBox then
    Cancel()
  end
end

local function InstallS3()
  if EventRegistry and ChatFrameEditBoxMixin and ChatFrameEditBoxMixin.OnPreSendText then
    EventRegistry:RegisterCallback("ChatFrame.OnEditBoxPreSendText", function(_, editBox)
      local ok, err = pcall(OnPreSend, editBox)
      if not ok then Print("|cffff4040Erreur S3 : " .. tostring(err) .. "|r") end
    end, "ChevaliersTranslators")
    s3Method = "rappel Blizzard ChatFrame.OnEditBoxPreSendText"
  elseif not issecretvalue then
    -- Client sans ce rappel et sans verrouillage : on enveloppe Entrée sur les boîtes de chat.
    for i = 1, (NUM_CHAT_WINDOWS or 10) do
      local eb = _G["ChatFrame" .. i .. "EditBox"]
      local orig = eb and eb:GetScript("OnEnterPressed")
      if orig then
        eb:SetScript("OnEnterPressed", function(self, ...)
          local ok, err = pcall(OnPreSend, self)
          if not ok then Print("|cffff4040Erreur S3 : " .. tostring(err) .. "|r") end
          return orig(self, ...)
        end)
      end
    end
    s3Method = "enveloppe OnEnterPressed (ancien client)"
  else
    s3Method = "indisponible sur ce client"
  end
  -- Brouillons, frappe et fermeture : lecture après coup (aucun script de la boîte n'est remplacé).
  for i = 1, (NUM_CHAT_WINDOWS or 10) do
    local eb = _G["ChatFrame" .. i .. "EditBox"]
    if eb and eb.HookScript then
      pcall(eb.HookScript, eb, "OnTextChanged", OnDraftChanged)
      pcall(eb.HookScript, eb, "OnEditFocusLost", OnBoxClosed)
      pcall(eb.HookScript, eb, "OnHide", OnBoxClosed)
    end
  end
end

local s3Driver = CreateFrame("Frame")
s3Driver:SetScript("OnUpdate", function()
  local now = GetTime()
  if draftDue and now >= draftDue then
    draftDue = nil
    if draftBox then
      local ok, err = pcall(SendDraft, draftBox)
      if not ok then Print("|cffff4040Erreur brouillon : " .. tostring(err) .. "|r") end
    end
  end
  local s = awaiting
  if s and s.reopenAt and now > s.reopenAt then
    -- Image suivante : le jeu a fini de fermer la boîte, on la rouvre.
    s.reopenAt = nil
    local ok, err = pcall(Reopen, s)
    if not ok then
      awaiting = nil
      Print("|cffff4040Erreur S3 (réouverture) : " .. tostring(err) .. "|r")
    end
  elseif s and s.reopened and now - s.at > PASTE_TIMEOUT then
    awaiting = nil
    s3Stats.timeouts = s3Stats.timeouts + 1
    SendState(STATE_CANCEL, s.id)
    local eb = s.box
    if eb and eb.IsShown and eb:IsShown() and eb:GetText() == "" then
      eb:SetText(s.text)
      Print("Traduction pas arrivée à temps : ton texte est remis dans la boîte (Entrée pour l'envoyer tel quel).")
    else
      Print("Traduction pas arrivée à temps : « " .. s.text .. " » n'a pas été envoyé.")
    end
  end
  if revertAt and now >= revertAt then
    SetDest("AUTO")
    Print("Trad revenu en automatique (60 s après ton dernier envoi en langue choisie).")
  end
end)

-- Bouton Trad sur la boîte de saisie et menu des langues -----------------------
local LANG_NAMES = {
  AUTO = "Automatique", FR = "Français", EN = "Anglais", DE = "Allemand", ES = "Espagnol", IT = "Italien",
  RU = "Russe", PT = "Portugais", NL = "Néerlandais", PL = "Polonais", CS = "Tchèque", SK = "Slovaque",
  HU = "Hongrois", RO = "Roumain", BG = "Bulgare", EL = "Grec", HR = "Croate", SR = "Serbe", SL = "Slovène",
  UK = "Ukrainien", SV = "Suédois", NO = "Norvégien", FI = "Finnois", DA = "Danois",
}
-- Langues courantes et mode automatique : en bas du menu (0.14.6). En haut, les régions, l'Europe
-- du Nord en premier.
local MAIN_LANGS = { "AUTO", "FR", "EN", "DE", "ES", "IT", "RU" }
local REGIONS = {
  { "Europe du Nord", { "SV", "NO", "DA", "FI" } },
  { "Europe de l'Est et du Sud-Est", { "PL", "CS", "SK", "HU", "RO", "BG", "EL", "HR", "SR", "SL", "UK" } },
  { "Europe de l'Ouest et du Sud", { "PT", "NL" } },
}
-- Ordre du clic sans menu (anciens clients) : même ordre que le menu (haut en bas).
local TRAD_LANGS = { "SV", "NO", "DA", "FI", "PL", "CS", "SK", "HU", "RO", "BG", "EL", "HR", "SR", "SL",
  "UK", "PT", "NL", "AUTO", "FR", "EN", "DE", "ES", "IT", "RU" }

-- Contenu du menu, décrit une fois pour les deux systèmes de menus du jeu. Un seul niveau : un
-- clic sur le bouton, un clic sur la langue (0.11). Régions en haut (Europe du Nord d'abord),
-- langues courantes et mode automatique en bas (0.14.6).
CT.TradMenu = function(add)
  add("title", "Langue du message envoyé")
  for i, r in ipairs(REGIONS) do
    if i > 1 then add("divider") end
    add("title", r[1])
    for _, c in ipairs(r[2]) do add("radio", LANG_NAMES[c], c) end
  end
  add("divider")
  add("title", "Saisie : langue détectée automatiquement")
  for _, c in ipairs(MAIN_LANGS) do add("radio", LANG_NAMES[c], c) end
end

local function OpenTradMenu(owner)
  if MenuUtil and MenuUtil.CreateContextMenu then
    -- Menus du jeu récents (même présentation que l'exemple : titres, boutons radio).
    MenuUtil.CreateContextMenu(owner, function(_, root)
      CT.TradMenu(function(kind, label, code)
        if kind == "title" then root:CreateTitle(label)
        elseif kind == "divider" then root:CreateDivider()
        elseif kind == "radio" then
          root:CreateRadio(label, function() return dest == code end, function()
            SetDest(code)
            -- Le choix ferme le menu.
            return MenuResponse and MenuResponse.Close
          end, code)
        end
      end)
    end)
    return true
  end
  if EasyMenu then
    -- Anciens menus déroulants.
    local items = {}
    CT.TradMenu(function(kind, label, code)
      if kind == "title" then items[#items + 1] = { text = label, isTitle = true, notCheckable = true }
      elseif kind == "radio" then
        items[#items + 1] = { text = label, checked = function() return dest == code end, func = function() SetDest(code) end }
      end
    end)
    CT.tradMenuFrame = CT.tradMenuFrame or CreateFrame("Frame", "ChevaliersTranslatorsTradMenu", UIParent, "UIDropDownMenuTemplate")
    EasyMenu(items, CT.tradMenuFrame, owner, 0, 0, "MENU")
    return true
  end
  return false
end

local function TradClick(self, button)
  -- Maj+clic gauche : traduire et envoyer tout de suite, comme Maj+Entrée (0.14.7).
  if button == "LeftButton" and IsShiftKeyDown() then
    local eb = self.ctEditBox or (ChatEdit_GetActiveWindow and ChatEdit_GetActiveWindow())
    if not StartSend(eb) then
      Print("Maj+clic sur Trad : écris d'abord un message dans la boîte de saisie.")
    end
    return
  end
  if button == "RightButton" then
    SetDest("AUTO")
  elseif not OpenTradMenu(self) then
    local i = 1
    for k, v in ipairs(TRAD_LANGS) do
      if v == dest then i = k end
    end
    SetDest(TRAD_LANGS[(i % #TRAD_LANGS) + 1])
  end
end

-- Bouton à part, à droite de la barre de saisie (0.12) : rattaché à la fenêtre de chat, il reste
-- visible et cliquable sans ouvrir la saisie, et un clic ne ferme rien.
local function AddTradButton(eb, cf)
  if not eb or eb.ctTrad then return end
  local b = CreateFrame("Button", nil, cf or (eb.GetParent and eb:GetParent()) or UIParent)
  b:SetSize(84, 22)
  b:SetPoint("LEFT", eb, "RIGHT", 4, 0)
  if b.SetFrameStrata then b:SetFrameStrata("MEDIUM") end
  local bg = b:CreateTexture(nil, "BACKGROUND")
  bg:SetAllPoints(b)
  bg:SetColorTexture(0, 0, 0, 0.65)
  local fs = b:CreateFontString(nil, "OVERLAY", "GameFontNormalSmall")
  fs:SetPoint("CENTER")
  b:SetFontString(fs)
  b.ctEditBox = eb
  b:RegisterForClicks("LeftButtonUp", "RightButtonUp")
  b:SetScript("OnClick", TradClick)
  b:SetScript("OnEnter", function(self)
    if not GameTooltip then return end
    local vers = dest == "AUTO" and "dans la langue de ton interlocuteur" or ("en " .. (LANG_NAMES[dest] or dest))
    GameTooltip:SetOwner(self, "ANCHOR_TOP")
    GameTooltip:AddLine("Chevaliers Translators")
    GameTooltip:AddLine("Traduire et envoyer " .. vers .. " : Maj+Entrée ou Maj+clic ici.", 1, 1, 1, true)
    GameTooltip:AddLine("Clic : choisir la langue. Clic droit : automatique.", 0.8, 0.8, 0.8)
    GameTooltip:Show()
  end)
  b:SetScript("OnLeave", function() if GameTooltip then GameTooltip:Hide() end end)
  b:SetText("Trad:" .. dest)
  eb.ctTrad = b
  tradButtons[#tradButtons + 1] = b
end

local function InstallTrad()
  for i = 1, (NUM_CHAT_WINDOWS or 10) do
    local ok, err = pcall(AddTradButton, _G["ChatFrame" .. i .. "EditBox"], _G["ChatFrame" .. i])
    if not ok then Print("|cffff4040Bouton Trad : " .. tostring(err) .. "|r") return end
  end
end

-- Police et position du chat (mesures pour le chat miroir) -------------------

local CHAT_ADDONS = { "ElvUI", "Prat-3.0", "Chattynator", "Glass", "ls_Glass", "WIM", "Chatter", "BasicChatMods" }

local function Num(v)
  v = tonumber(v) or 0
  if v == floor(v) then return tostring(v) end
  return format("%.1f", v)
end

-- "Fonts\ARIALN.TTF 14 ombre 1,-1" pour un cadre ou un objet police.
local function FontDesc(obj)
  if type(obj) ~= "table" or type(obj.GetFont) ~= "function" then return "inconnue" end
  local ok, path, size, flags = pcall(obj.GetFont, obj)
  if not ok or not path then return "inconnue" end
  local desc = tostring(path) .. " " .. Num(size)
  if flags and flags ~= "" then desc = desc .. " " .. tostring(flags) end
  if type(obj.GetShadowOffset) == "function" then
    local okS, sx, sy = pcall(obj.GetShadowOffset, obj)
    if okS and ((sx or 0) ~= 0 or (sy or 0) ~= 0) then
      desc = desc .. format(" ombre %s,%s", Num(sx), Num(sy))
    end
  end
  return desc
end

local function ChatAddOns()
  local found = {}
  for _, name in ipairs(CHAT_ADDONS) do
    if IsLoadedCompat(name) then found[#found + 1] = name end
  end
  return #found > 0 and table.concat(found, ", ") or "aucun (chat Blizzard)"
end

-- Taille et position en pixels physiques (768 unités de hauteur à l'échelle 1).
local function FrameGeometry(f)
  local ok, l, b, w, h = pcall(function() return f:GetLeft(), f:GetBottom(), f:GetWidth(), f:GetHeight() end)
  if not ok or not (l and b and w and h) then return "" end
  local _, ph = GetPhysicalScreenSize()
  local k = (f.GetEffectiveScale and f:GetEffectiveScale() or 1) * (ph or 768) / 768
  return format(" - %dx%d px, coin bas-gauche (%d, %d)",
    floor(w * k + 0.5), floor(h * k + 0.5), floor(l * k + 0.5), floor(b * k + 0.5))
end

local function CmdFont()
  Print(format("Client %s - addons de chat : %s", tostring(GetLocale and GetLocale() or "?"), ChatAddOns()))
  Print("Police de référence (ChatFontNormal) : " .. FontDesc(ChatFontNormal))
  local count = 0
  local C = Constants and Constants.ChatFrameConstants
  local max = (C and C.MaxChatWindows or NUM_CHAT_WINDOWS or 10) + 10 -- + fenêtres temporaires
  for i = 1, max do
    local cf = _G["ChatFrame" .. i]
    local name, shown, docked
    if GetChatWindowInfo then
      local info = { pcall(GetChatWindowInfo, i) }
      if info[1] then name, shown, docked = info[2], info[8], info[10] end
    end
    if type(cf) == "table" and ((cf.IsShown and cf:IsShown()) or shown or docked) then
      count = count + 1
      local combat = IsCombatLog and IsCombatLog(cf)
      Print(format("ChatFrame%d « %s »%s : %s%s", i, tostring(name or "?"),
        combat and " (journal de combat, lecture seule)" or "", FontDesc(cf), FrameGeometry(cf)))
    end
  end
  if count == 0 then Print("Aucune fenêtre de chat Blizzard visible (addon de chat qui remplace les fenêtres ?)") end
end

-- Commandes -----------------------------------------------------------------

local function CmdInfo()
  local _, _, _, iface = GetBuildInfo()
  local B = CT.Banner
  Print(format("Interface %s - projet %s", tostring(iface), tostring(WOW_PROJECT_ID)))
  Print(format("Écran %sx%s px - bloc %d px - %d colonnes - charge utile %d octets/trame",
    tostring(B.pw), tostring(B.ph), B.block, B.cols, MaxPayload()))
  if B.error then Print("|cffff4040Erreur bandeau : " .. B.error .. "|r") end
  Print(format("Cadence %.0f trames/s - file %d - émises %d - perdues %d - chat %d - secrets ignorés %d",
    1 / hold, qtail - qhead + 1, stats.sent, stats.dropped, stats.chat, stats.secret))
  local function has(v) return v and "oui" or "non" end
  Print(format("API : issecretvalue %s, verrouillage chat %s (actif : %s), EventRegistry %s, C_AddOns %s",
    has(issecretvalue), has(C_ChatInfo and C_ChatInfo.InChatMessagingLockdown), has(InLockdown()),
    has(EventRegistry), has(C_AddOns and C_AddOns.LoadAddOn)))
  local qv
  for i = 1, NumAddOns() do
    local name = AddOnName(i)
    if name and name:lower():find("questvoice", 1, true) then
      qv = name .. (IsLoadedCompat(name) and " (chargé)" or " (non chargé)")
      break
    end
  end
  Print("QuestVoice : " .. (qv or "absent") .. " - bandeau CT en bas à droite.")
  local fullName, fullRealm = SafeCall(UnitFullName, "player")
  Print(format("Noms : UnitName %s - GetUnitName %s - UnitFullName %s %s - appris %s",
    tostring(UnitName("player")), tostring(SafeCall(GetUnitName, "player", true)),
    tostring(fullName), tostring(fullRealm), tostring(selfName or "pas encore")))
  if CT.Mirror then
    local st = CT.Mirror.stats
    Print(format("Chat miroir : %s - descriptions %d, lignes %d, retours au texte officiel %d",
      CT.Mirror.mode, st.views, st.lines, st.official))
  end
  Print(format("Écran : %s - dispositif télévision %s (réglage %s)", CT.screenTV and "télévision" or "écran de PC",
    CT.CoverMode() and "actif" or "inactif", tostring(ChevaliersTranslatorsDB and ChevaliersTranslatorsDB.tele or "auto")))
  Print(format("Police du chat : %s - client %s - détail : /chevaliers font",
    FontDesc(DEFAULT_CHAT_FRAME), tostring(GetLocale and GetLocale() or "?")))
  local _, langName, langSource = ChatLanguage()
  Print(format("Langue d'envoi : %s (%s)", langName, langSource))
  Print(format("S3 : %s - cible %s - demandes %d, envois %d, annulés %d, trop lents %d, brouillons %d%s",
    s3Method, dest, s3Stats.requests, s3Stats.sent, s3Stats.cancelled, s3Stats.timeouts, s3Stats.drafts,
    awaiting and " - en attente du collage" or ""))
  Print(format("Traduction du chat : %s%s", chatOn and "active" or "coupée",
    CT.liveSeen and (" - application " .. CT.liveSeen) or " - application non vue à la connexion"))
end

-- Traduction du chat : relais des messages vers l'application et chat miroir. Active d'office à
-- la connexion quand l'application tourne ; /chevaliers off la coupe durablement.
local function SetTrad(on, how)
  chatOn = on
  CT.waiting = nil
  if CT.Mirror then CT.Mirror:Command(on and "on" or "off", true) end
  if on and CT.SendZone then CT.SendZone() end
  if how == "connexion" then
    Print("Traduction du chat active (application " .. tostring(CT.liveSeen) .. ") : messages étrangers traduits, Maj+Entrée pour envoyer traduit, Trad:" .. dest .. ".")
  elseif on then
    Print("Traduction du chat active : messages étrangers traduits dans le chat. Envoi : Maj+Entrée, Trad:" .. dest .. ".")
  else
    Print("Traduction du chat arrêtée : chat officiel" .. (how == "commande" and " (gardé aux prochaines connexions, /chevaliers on pour la remettre)." or "."))
  end
end

-- Diagnostic (retail : canaux absents du miroir) : état de chaque fenêtre de chat et des
-- messages reçus, affiché ici et recopié dans la console de l'application.
local function Note(text)
  Print(text)
  local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
  Enqueue(CT.FRAME_NOTE, p, #p)
end

local function CmdDiag()
  local version, build, _, iface = GetBuildInfo()
  Note(format("diag : client %s.%s interface %s projet %s, langue %s, miroir %s, trad %s, verrouillage %s, issecretvalue %s",
    tostring(version), tostring(build), tostring(iface), tostring(WOW_PROJECT_ID), tostring(GetLocale and GetLocale()),
    CT.Mirror and CT.Mirror.mode or "?", chatOn and "on" or "off", tostring(InLockdown()), issecretvalue and "oui" or "non"))
  local C = Constants and Constants.ChatFrameConstants
  for i = 1, (C and C.MaxChatWindows or NUM_CHAT_WINDOWS or 10) + 10 do
    local cf = _G["ChatFrame" .. i]
    if type(cf) == "table" and cf.IsShown then
      local n, secret, sample = 0, 0, nil
      for _, fs in ipairs(cf.visibleLines or {}) do
        local info = fs.messageInfo
        if info then
          n = n + 1
          if issecretvalue and issecretvalue(info.message) then
            secret = secret + 1
          elseif not sample and type(info.message) == "string" then
            sample = info.message:gsub("|", "!"):sub(1, 60)
          end
        end
      end
      local c = cf.FontStringContainer
      local name = "?"
      if GetChatWindowInfo then
        local ok, nm = pcall(GetChatWindowInfo, i)
        if ok and nm then name = nm end
      end
      Note(format("diag : ChatFrame%d « %s » montré %s visible %s, conteneur %s alpha %s, lignes %d dont secrètes %d%s",
        i, tostring(name), tostring(cf:IsShown()), tostring(cf.IsVisible and cf:IsVisible()), c and "oui" or "non",
        c and c.GetAlpha and format("%.2f", c:GetAlpha()) or "-", n, secret, sample and (" - « " .. sample .. " »") or ""))
    end
  end
  local list = {}
  for e, v in pairs(eventCounts) do list[#list + 1] = format("%s %d/%d", (e:gsub("CHAT_MSG_", "")), v.n, v.secret) end
  table.sort(list)
  Note("diag : messages reçus (total/secrets) : " .. (#list > 0 and table.concat(list, ", ") or "aucun"))
end

local function CmdTest(a, b)
  local n = tonumber(a) or 1000
  local hz = tonumber(b) or 20
  if n < 1 or n > 65536 then Print("Nombre de trames entre 1 et 65536.") return end
  if hz < 1 or hz > 60 then Print("Cadence entre 1 et 60 trames/s.") return end
  hold = 1 / hz
  test = { next = 0, total = n, t0 = Now() }
  Print(format("Test : %d trames à %d trames/s, soit %.1f s. Lance d'abord le décodeur.", n, hz, n / hz))
end

-- Commandes du joueur, et outils de développement (tests de la phase 0) acceptés tels quels mais
-- absents de l'aide, listés par « /chevaliers dev ».
-- Guide d'installation (0.14.6) : petit panneau affiché une fois, à la première utilisation de
-- l'addon (et par /chevaliers guide), avec la bannière et le logo. Il dit où se trouve
-- l'application compagnon et comment la lancer. Fermeture par la croix ou au bout de 60 s.
local MEDIA = "Interface\\AddOns\\ChevaliersTranslators\\Media\\"
local GUIDE_TEXT =
  "Bienvenue dans Chevaliers Translators !\n\n" ..
  "L'addon affiche le bandeau en bas de l'écran ; c'est l'application compagnon qui traduit le " ..
  "chat, sur ton PC (aucun serveur en ligne).\n\n" ..
  "|cffe6c77e1.|r L'application se trouve dans le dossier de l'addon : " ..
  "|cffffffffChevaliersTranslators.exe|r. Double-clique dessus pour la lancer.\n" ..
  "|cffe6c77e2.|r Lance ensuite World of Warcraft : la traduction démarre à la connexion du " ..
  "personnage (si l'application a été lancée après, fais /reload).\n\n" ..
  "L'application n'est pas encore signée numériquement (une demande est en cours auprès de la " ..
  "SignPath Foundation) : Windows peut la présenter comme venant d'un éditeur inconnu. Le code " ..
  "est ouvert et vérifiable :\n|cff79b8ffgithub.com/Heaumite/chevaliers-translators|r\n\n" ..
  "Tu peux revoir ce guide avec |cffe6c77e/chevaliers guide|r."

local guideFrame
local function BuildGuide()
  if guideFrame then return guideFrame end
  local f = CreateFrame("Frame", "ChevaliersTranslatorsGuide", UIParent)
  f:SetFrameStrata("DIALOG")
  f:SetSize(500, 420)
  f:SetPoint("CENTER")
  f:EnableMouse(true)
  pcall(f.SetClampedToScreen, f, true)
  if f.SetMovable then
    f:SetMovable(true)
    pcall(f.RegisterForDrag, f, "LeftButton")
    f:SetScript("OnDragStart", f.StartMoving)
    f:SetScript("OnDragStop", f.StopMovingOrSizing)
  end
  -- Fond sombre et cadre or (couleurs unies : pas de dépendance aux modèles de cadre du jeu).
  local bg = f:CreateTexture(nil, "BACKGROUND")
  bg:SetAllPoints(f)
  bg:SetColorTexture(0.055, 0.051, 0.039, 0.97)
  for _, s in ipairs({ "TOP", "BOTTOM", "LEFT", "RIGHT" }) do
    local e = f:CreateTexture(nil, "BORDER")
    e:SetColorTexture(0.788, 0.647, 0.353, 1)
    if s == "TOP" then e:SetPoint("TOPLEFT"); e:SetPoint("TOPRIGHT"); e:SetHeight(2)
    elseif s == "BOTTOM" then e:SetPoint("BOTTOMLEFT"); e:SetPoint("BOTTOMRIGHT"); e:SetHeight(2)
    elseif s == "LEFT" then e:SetPoint("TOPLEFT"); e:SetPoint("BOTTOMLEFT"); e:SetWidth(2)
    else e:SetPoint("TOPRIGHT"); e:SetPoint("BOTTOMRIGHT"); e:SetWidth(2) end
  end
  -- Bannière de l'addon en haut.
  local ban = f:CreateTexture(nil, "ARTWORK")
  ban:SetPoint("TOP", f, "TOP", 0, -8)
  ban:SetSize(468, 117)
  ban:SetTexture(MEDIA .. "Banniere")
  -- Logo et titre.
  local logo = f:CreateTexture(nil, "ARTWORK")
  logo:SetPoint("TOPLEFT", f, "TOPLEFT", 16, -134)
  logo:SetSize(40, 40)
  logo:SetTexture(MEDIA .. "Logo")
  local title = f:CreateFontString(nil, "OVERLAY", "GameFontNormalLarge")
  title:SetPoint("LEFT", logo, "RIGHT", 10, 0)
  title:SetText("Guide d'installation")
  -- Croix de fermeture.
  local close = CreateFrame("Button", nil, f, "UIPanelCloseButton")
  close:SetPoint("TOPRIGHT", f, "TOPRIGHT", -2, -2)
  -- Texte du guide.
  local body = f:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
  body:SetPoint("TOPLEFT", f, "TOPLEFT", 20, -186)
  body:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -20, 16)
  body:SetJustifyH("LEFT")
  body:SetJustifyV("TOP")
  pcall(body.SetSpacing, body, 3)
  body:SetText(GUIDE_TEXT)
  -- Fermeture automatique au bout de 60 s après l'affichage.
  f:SetScript("OnShow", function(self) self.hideAt = GetTime() + 60 end)
  f:SetScript("OnUpdate", function(self)
    if self.hideAt and GetTime() >= self.hideAt then self.hideAt = nil self:Hide() end
  end)
  guideFrame = f
  return f
end

local function ShowGuide()
  if ChevaliersTranslatorsDB then ChevaliersTranslatorsDB.guideSeen = true end
  local ok, f = pcall(BuildGuide)
  if ok and f then f:Show() end
end
CT.ShowGuide = ShowGuide

local function Help()
  Print("Commandes : /chevaliers on | off (traduction du chat), langue auto | xx (langue de l'envoi " ..
    "traduit, aussi le bouton Trad), info (état), diag (rapport recopié dans le journal de l'application).")
  Print("Envoi traduit : Maj+Entrée dans la boîte de saisie. Entrée seule envoie ton texte tel quel.")
  Print("Proposition d'un modèle supérieur (panneau sur l'écran) : /chevaliers monter pour l'essayer, /chevaliers non pour ne plus le proposer.")
  Print("Guide d'installation de l'application : /chevaliers guide.")
end

local function DevHelp()
  Print("Développement : test [n] [hz], stop, ping [texte], chat on|off, block [px], rate [hz], font, " ..
    "miroir on|calage|off|sync|echantillon, perf on|off, essai [low|low-cpu|4b|4b-cpu|7b|stop|fermer|aide], " ..
    "tele auto|on|off (dispositif télévision)")
end

SLASH_CHEVALIERS1 = "/chevaliers"
SlashCmdList.CHEVALIERS = function(msg)
  local cmd, rest = (msg or ""):match("^%s*(%S*)%s*(.-)%s*$")
  cmd = (cmd or ""):lower()
  -- « /chevaliers on|off » : la traduction du chat (comme « trad on|off »).
  if cmd == "on" or cmd == "off" then
    rest, cmd = cmd, "trad"
  elseif cmd == "langue" then
    cmd = "lang"
  end
  local a, b = rest:match("^(%S*)%s*(%S*)")
  if cmd == "" or cmd == "aide" or cmd == "help" then
    Help()
  elseif cmd == "dev" then
    DevHelp()
  elseif cmd == "monter" or cmd == "non" then
    -- Réponse à une proposition de l'application ; elle part trois fois (une trame ne reste que
    -- 50 ms à l'écran), avec un numéro pour n'être comptée qu'une fois.
    local text = cmd .. "\t" .. floor(GetTime() * 1000) % 100000000
    local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
    for _ = 1, 3 do Enqueue(CT.FRAME_CMD, p, #p) end
    if cmd == "monter" then
      Print("Demande transmise : le modèle proposé sera essayé hors combat ; en cas de souci, la traduction revient seule au modèle actuel.")
    else
      Print("Proposition écartée : ce modèle ne te sera plus proposé.")
    end
  elseif cmd == "info" then
    CmdInfo()
  elseif cmd == "guide" then
    ShowGuide()
  elseif cmd == "tele" or cmd == "télé" then
    a = (a or ""):lower()
    if a == "auto" or a == "on" or a == "off" then
      -- Pris en compte à la description suivante des fenêtres de chat (20 fois par seconde).
      ChevaliersTranslatorsDB.tele = a ~= "auto" and a or nil
    end
    Print(format("Dispositif télévision : %s (réglage %s ; écran vu par l'application : %s).",
      CT.CoverMode() and "actif, le chat traduit recouvre celui du jeu" or "inactif, le chat du jeu est caché sous le chat traduit",
      tostring(ChevaliersTranslatorsDB.tele or "auto"), CT.screenTV and "télévision" or "écran de PC"))
  elseif cmd == "test" then
    CmdTest(a, b)
  elseif cmd == "stop" then
    test = nil
    queue, qhead, qtail = {}, 1, 0
    prio, phead, ptail = {}, 1, 0
    low, lhead, ltail = {}, 1, 0
    Print("Arrêt : file vidée.")
  elseif cmd == "ping" then
    local text = rest ~= "" and rest or "Bonjour depuis WoW"
    local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
    Enqueue(CT.FRAME_PING, p, #p)
    Print("Ping envoyé : " .. text)
  elseif cmd == "chat" then
    chatOn = (a:lower() ~= "off")
    Print("Relais du chat : " .. (chatOn and "activé" or "désactivé"))
  elseif cmd == "trad" then
    local on = (a:lower() ~= "off")
    ChevaliersTranslatorsDB.tradOff = (not on) or nil
    SetTrad(on, "commande")
  elseif cmd == "diag" then
    local ok, err = pcall(CmdDiag)
    if not ok then Print("|cffff4040Erreur diag : " .. tostring(err) .. "|r") end
  elseif cmd == "block" then
    local B = tonumber(a)
    if not B or B < 2 or B > 16 then Print("Taille de bloc entre 2 et 16 px.") return end
    ChevaliersTranslatorsDB.block = B
    CT.Banner.block = B
    CT.Banner:Layout()
    Print(format("Bloc %d px : %d colonnes, %d octets/trame.", B, CT.Banner.cols, MaxPayload()))
  elseif cmd == "perf" then
    CT.perfOn = (a:lower() ~= "off")
    ChevaliersTranslatorsDB.perf = CT.perfOn
    Print("Images par seconde et à-coups envoyés à l'application : " .. (CT.perfOn and "oui (2 fois par seconde, gardé après /reload)" or "non"))
  elseif cmd == "essai" or cmd == "modele" then
    -- Test d'un modèle de traduction lancé depuis le jeu : l'outil (ct-outils tests-en-jeu) lit
    -- la demande dans le bandeau et affiche le verdict par-dessus WoW.
    local what = (a or ""):lower()
    if what == "aide" or what == "help" then
      Print("Tests en jeu des modèles (l'outil doit tourner : ct-outils tests-en-jeu) :")
      Print("  /chevaliers essai - série complète, environ 20 min, en restant dans WoW ; l'application se ferme à la fin")
      Print("  /chevaliers essai low | low-cpu | 4b | 4b-cpu | 7b - un seul test (environ 4 min)")
      Print("  /chevaliers essai stop - arrête ; /chevaliers essai fermer - masque le panneau")
      return
    end
    if what == "" then what = "tout" end
    if not CT.perfOn then
      CT.perfOn = true
      ChevaliersTranslatorsDB.perf = true
    end
    -- Une trame ne reste que 50 ms à l'écran : la demande part trois fois, avec un numéro
    -- pour que l'application ne la compte qu'une fois.
    local text = "essai " .. what .. "\t" .. floor(GetTime() * 1000) % 100000000
    local p = { string.byte(text, 1, math.min(#text, MaxPayload())) }
    for _ = 1, 3 do Enqueue(CT.FRAME_CMD, p, #p) end
    if what == "stop" then
      Print("Arrêt du test demandé à l'application.")
    elseif what == "fermer" then
      Print("Panneau masqué.")
    elseif what == "tout" then
      Print("Série complète demandée à l'application : ne bouge plus pendant environ 15 min ; le panneau en haut à droite suit les tests, l'application se ferme après le bilan.")
    else
      Print("Essai « " .. what .. " » demandé à l'application : ne bouge plus, le panneau en haut à droite suit le test.")
    end
  elseif cmd == "miroir" or cmd == "mirror" then
    if CT.Mirror then CT.Mirror:Command(a) else Print("Mirror.lua absent : réinstaller l'addon.") end
  elseif cmd == "font" or cmd == "police" then
    local ok, err = pcall(CmdFont)
    if not ok then Print("|cffff4040Erreur : " .. tostring(err) .. "|r") end
  elseif cmd == "lang" then
    if a ~= "" then SetDest(a:upper():sub(1, 5)) end
    Print("Langue de l'envoi traduit (Maj+Entrée) : " .. dest ..
      (dest == "AUTO" and " (langue de ton interlocuteur, devinée par l'application)" or ""))
  elseif cmd == "rate" then
    local hz = tonumber(a)
    if hz and hz >= 1 and hz <= 60 then hold = 1 / hz end
    Print(format("Cadence : %.0f trames/s.", 1 / hold))
  else
    Help()
  end
end

-- Démarrage -----------------------------------------------------------------

local boot = CreateFrame("Frame")
boot:RegisterEvent("PLAYER_LOGIN")
pcall(boot.RegisterEvent, boot, "DISPLAY_SIZE_CHANGED")
pcall(boot.RegisterEvent, boot, "UI_SCALE_CHANGED")
boot:SetScript("OnEvent", function(self, event)
  if event == "PLAYER_LOGIN" then
    ChevaliersTranslatorsDB = ChevaliersTranslatorsDB or {}
    -- Trad commence toujours en automatique.
    InstallS3()
    InstallTrad()
    if not CT.Banner:Init(ChevaliersTranslatorsDB.block or 4) then
      Print("|cffff4040Bandeau indisponible : " .. tostring(CT.Banner.error) .. "|r")
      return
    end
    -- Mesures pour l'application gardées après /reload et redémarrage.
    CT.perfOn = ChevaliersTranslatorsDB.perf and true or false
    -- L'application réécrit toutes les 2 s un petit addon (ChevaliersTranslators_Live) avec
    -- l'heure (t, 0 quand elle est quittée) : relu à chaque connexion et /reload, il dit si elle
    -- tourne. Son âge est pris au moment où le jeu a lu le fichier (lu), pas maintenant : l'écran
    -- de chargement dure parfois plus d'une minute sur retail, la présence semblait périmée et
    -- le « /reload » était demandé alors que l'application tournait (0.13.0).
    local live = CHEVALIERS_LIVE
    local now = (time and time()) or (GetServerTime and GetServerTime())
    local read = type(live) == "table" and type(live.lu) == "number" and live.lu or now
    local fresh = type(live) == "table" and type(live.t) == "number" and live.t > 0 and read
      and math.abs(read - live.t) <= 20
    CT.liveSeen = fresh and tostring(live.v or "?") or nil
    CT.screenTV = fresh and live.e == 1 or false
    if CT.liveSeen and CT.VERSION and live.v ~= CT.VERSION then
      Print("|cffff8000Application " .. tostring(live.v) .. " et addon " .. tostring(CT.VERSION) .. " : relance l'application (elle met l'addon à jour), puis /reload.|r")
    end
    -- Première utilisation : guide d'installation affiché une fois, peu après la connexion (pas
    -- pendant l'écran de chargement). C_Timer absent (hors jeu) : pas de guide.
    if not ChevaliersTranslatorsDB.guideSeen and C_Timer and C_Timer.After then
      C_Timer.After(3, function() pcall(ShowGuide) end)
    end
    if ChevaliersTranslatorsDB.tradOff then
      Print("Prêt - traduction du chat coupée (/chevaliers on pour la remettre).")
    elseif fresh then
      SetTrad(true, "connexion")
      if CT.CoverMode() then
        Print("Écran de télévision : le chat traduit recouvre le chat du jeu (/chevaliers tele off pour le cacher comme sur un écran de PC).")
      end
    else
      CT.waiting = true
      Print("Prêt - l'application Chevaliers Translators n'est pas lancée : chat officiel. Lance-la, puis /reload pour traduire.")
    end
  else
    CT.Banner:Layout()
  end
end)

-- Images par seconde (test en jeu des modèles de traduction) ----------------

-- Deux fois par seconde : images par seconde (x10) et durée de la plus longue image de la
-- demi-seconde, en millisecondes (les à-coups annoncent une carte graphique saturée).
local perfFrame = CreateFrame("Frame")
local perfAcc, perfWorst = 0, 0
perfFrame:SetScript("OnUpdate", function(_, elapsed)
  if not CT.perfOn then perfAcc, perfWorst = 0, 0 return end
  elapsed = elapsed or 0
  perfAcc = perfAcc + elapsed
  if elapsed > perfWorst then perfWorst = elapsed end
  if perfAcc < 0.5 then return end
  perfAcc = 0
  local fps = GetFramerate and GetFramerate() or 0
  local v = floor(fps * 10 + 0.5)
  if v > 65535 then v = 65535 end
  local ms = floor(perfWorst * 1000 + 0.5)
  if ms > 65535 then ms = 65535 end
  perfWorst = 0
  Enqueue(CT.FRAME_PERF, { floor(v / 256), v % 256, floor(ms / 256), ms % 256 }, 4)
end)
