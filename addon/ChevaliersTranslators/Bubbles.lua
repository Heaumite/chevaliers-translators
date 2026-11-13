-- Chevaliers Translators
-- Bubbles.lua : bulles de dialogue au-dessus des joueurs traduites (0.15).
-- Dix fois par seconde, l'addon repère les bulles visibles (Dire, Crier, Groupe, Raid...),
-- retrouve le message relayé que chacune affiche, et envoie à l'application sa place à l'écran,
-- sa police et sa couleur. L'application dessine dessus la traduction, précédée de l'acronyme de
-- la langue d'origine (« [DE] Bonsoir à tous ! »). Une bulle dans la langue du client reste telle
-- quelle (l'application n'a rien à dessiner).
-- Le jeu protège les bulles en donjon, raid, champ de bataille et arène (illisibles pour un
-- addon) : elles y restent telles quelles. Bulles cachées sous une fenêtre du jeu (carte,
-- sacs...) : pas traduites.
-- 0.16.3 : la bulle de Blizzard est effacée sous la nôtre (plus de double bulle) : l'application
-- dessine toute la bulle, traduite ou non (texte d'origine si rien n'est à traduire). Elle revient
-- telle quelle en combat, chat verrouillé, miroir coupé ou /chevaliers bulles off. Le texte de
-- chaque bulle part aussi avec sa place, tant qu'il y a de la place dans la trame : l'application
-- dessine la bulle même si elle a manqué le message.
-- 0.16.4 : bulle aux couleurs de la classe de celui qui parle (bord et teinte), avec l'icône de sa
-- spécialisation quand elle est connue : la sienne, et celle des joueurs proches ou du groupe,
-- demandée au jeu par une inspection discrète (hors combat, une à la fois, jamais pendant que la
-- fenêtre d'inspection est ouverte), gardée pour la session.
-- /chevaliers bulles on | off

local _, CT = ...
CT = CT or {}
local floor, min, max = math.floor, math.min, math.max
local format = string.format

local POLL = 0.1      -- lecture des bulles : 10 fois par seconde
local KEEP = 60       -- message gardé une minute pour retrouver sa bulle
local REFRESH = 2     -- place renvoyée même immobile (l'application oublie une bulle sans nouvelles)
local MOVE = 3        -- pixels : en dessous, la bulle est considérée immobile
local MAX = 8         -- bulles par trame

-- Événements dont le message peut s'afficher dans une bulle.
local BUBBLE_EVENTS = {
  CHAT_MSG_SAY = true, CHAT_MSG_YELL = true,
  CHAT_MSG_PARTY = true, CHAT_MSG_PARTY_LEADER = true,
  CHAT_MSG_RAID = true, CHAT_MSG_RAID_LEADER = true,
  CHAT_MSG_INSTANCE_CHAT = true, CHAT_MSG_INSTANCE_CHAT_LEADER = true,
}

local TEXT_MAX = 120    -- octets de texte au plus par bulle dans la trame

local INSPECT_EVERY = 2     -- secondes entre deux inspections
local INSPECT_WAIT = 4      -- réponse attendue ce temps

local B = {
  recent = {},              -- texte normalisé -> { id, at, guid }
  specs = {},               -- GUID -> numéro de spécialisation (inspection)
  asked = {},               -- GUID -> moment de la dernière inspection demandée
  inspectAt = -100,
  sentSig = "", sentAt = -100,
  stats = { frames = 0, matched = 0, unknown = 0, hidden = 0, errors = 0, replaced = 0 },
  -- Cadres de bulles de Blizzard effacés (0.16.3) : rendus visibles dès qu'ils ne sont plus à nous.
  erased = setmetatable({}, { __mode = "k" }),
}
CT.Bubbles = B

local function IsSecret(v) return issecretvalue ~= nil and issecretvalue(v) or false end

-- Texte comparable entre le message reçu et celui de la bulle : sans couleurs, liens, icônes ni
-- repères de raid ({rt1}, {étoile}), espaces simplifiés.
local function Norm(s)
  if type(s) ~= "string" or IsSecret(s) then return nil end
  s = s:gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", "")
  s = s:gsub("|H.-|h(.-)|h", "%1"):gsub("|T.-|t", ""):gsub("|A.-|a", "")
  s = s:gsub("{[^}]*}", ""):gsub("%s+", " ")
  s = s:match("^%s*(.-)%s*$")
  if s == "" then return nil end
  return s
end
B.Norm = Norm

function B:Enabled()
  return not (ChevaliersTranslatorsDB and ChevaliersTranslatorsDB.bubbles == false)
end

-- Message relayé (Core.lua) : retenu pour retrouver sa bulle (les siens compris, 0.16.1 : un
-- message du joueur traduit dans le chat l'est aussi dans sa bulle). `skip` : message à ne pas
-- chercher dans une bulle (chuchotement envoyé).
function B:Note(id, event, text, skip, guid)
  if skip or not BUBBLE_EVENTS[event] then return end
  local key = Norm(text)
  if not key then return end
  local now = GetTime()
  self.recent[key] = { id = id, at = now, guid = guid }
  -- Ménage de temps en temps.
  if (self.pruneAt or 0) < now - 10 then
    self.pruneAt = now
    for k, e in pairs(self.recent) do
      if now - e.at > KEEP then self.recent[k] = nil end
    end
  end
end

-- Message d'une bulle : son numéro de relais (texte identique, ou même début s'il est long), et
-- l'identifiant de celui qui parle.
function B:Lookup(text)
  local key = Norm(text)
  if not key then return nil end
  local e = self.recent[key]
  if e then return e.id, e.guid end
  if #key >= 24 then
    local head = key:sub(1, 24)
    local best
    for k, v in pairs(self.recent) do
      if k:sub(1, 24) == head and (not best or v.at > best.at) then best = v end
    end
    if best then return best.id, best.guid end
  end
end

-- Couleur de la classe d'un joueur (0 à 1), d'après son identifiant.
local function ClassColor(guid)
  if not (guid and GetPlayerInfoByGUID) then return nil end
  local ok, _, class = pcall(GetPlayerInfoByGUID, guid)
  if not ok or type(class) ~= "string" or IsSecret(class) then return nil end
  local c
  if C_ClassColor and C_ClassColor.GetClassColor then
    local okC, col = pcall(C_ClassColor.GetClassColor, class)
    if okC then c = col end
  end
  c = c or (RAID_CLASS_COLORS and RAID_CLASS_COLORS[class])
  if type(c) ~= "table" or type(c.r) ~= "number" then return nil end
  return c.r, c.g, c.b
end
B.ClassColor = ClassColor

-- Unité du jeu qui désigne ce joueur maintenant (cible, groupe, plaques des joueurs proches).
local function UnitOf(guid)
  local function same(u)
    local g = UnitGUID and UnitGUID(u)
    return type(g) == "string" and not IsSecret(g) and g == guid
  end
  for _, u in ipairs({ "target", "mouseover", "focus" }) do
    if same(u) then return u end
  end
  local raid = IsInRaid and IsInRaid()
  for i = 1, raid and 40 or 4 do
    local u = (raid and "raid" or "party") .. i
    if same(u) then return u end
  end
  for i = 1, 40 do
    local u = "nameplate" .. i
    if same(u) then return u end
  end
end

-- Icône (numéro de fichier) d'une spécialisation.
local function SpecIconOf(specID)
  if not (specID and GetSpecializationInfoByID) then return nil end
  local ok, _, _, _, icon = pcall(GetSpecializationInfoByID, specID)
  if not ok or IsSecret(icon) then return nil end
  if type(icon) == "string" and GetFileIDFromPath then icon = GetFileIDFromPath(icon) end
  return type(icon) == "number" and icon > 0 and icon or nil
end

-- Spécialisation de celui qui parle : la sienne tout de suite ; celle d'un autre après une
-- inspection (demandée ici, au plus une toutes les 2 s, hors combat).
local function SpecIcon(guid, now)
  if not guid then return nil end
  local me = UnitGUID and UnitGUID("player")
  if type(me) == "string" and not IsSecret(me) and me == guid then
    local idx = GetSpecialization and GetSpecialization()
    if type(idx) == "number" and GetSpecializationInfo then
      local ok, id = pcall(GetSpecializationInfo, idx)
      if ok and type(id) == "number" then return SpecIconOf(id) end
    end
    return nil
  end
  if B.specs[guid] then return SpecIconOf(B.specs[guid]) end
  if now - B.inspectAt < INSPECT_EVERY or (B.asked[guid] and now - B.asked[guid] < 60) then return nil end
  if not (NotifyInspect and CanInspect and GetInspectSpecialization) then return nil end
  if (InCombatLockdown and InCombatLockdown()) or (InspectFrame and InspectFrame.IsShown and InspectFrame:IsShown()) then return nil end
  local unit = UnitOf(guid)
  if not unit then return nil end
  local okC, can = pcall(CanInspect, unit)
  if not okC or IsSecret(can) or not can then return nil end
  B.inspectAt, B.asked[guid] = now, now
  B.pending = { guid = guid, unit = unit, at = now }
  pcall(NotifyInspect, unit)
  return nil
end
B.SpecIcon = SpecIcon

-- Réponse d'une inspection : la spécialisation est gardée pour la session.
function B:OnInspectReady(guid)
  local p = self.pending
  if not (p and type(guid) == "string") or IsSecret(guid) then return end
  if guid ~= p.guid then return end
  self.pending = nil
  local ok, spec = pcall(GetInspectSpecialization, p.unit)
  if ok and type(spec) == "number" and not IsSecret(spec) and spec > 0 then self.specs[guid] = spec end
end

-- Texte d'une bulle (selon les versions du jeu : champ String du cadre enfant, ou première
-- chaîne de texte trouvée).
local function FontStringOf(bubble)
  local holder = bubble.GetChildren and bubble:GetChildren()
  local fs = holder and holder.String
  local function scan(f)
    if not (f and f.GetRegions) then return nil end
    for _, r in ipairs({ f:GetRegions() }) do
      if r.GetObjectType and r:GetObjectType() == "FontString" then return r end
    end
  end
  return fs or scan(holder) or scan(bubble)
end

-- Cadre visible d'une bulle (fond, bord, pointe et texte) : celui que Blizzard affiche.
local function VisualOf(bubble)
  local holder = bubble.GetChildren and bubble:GetChildren()
  return (type(holder) == "table" and holder.SetAlpha and holder) or bubble
end

-- La bulle de Blizzard s'efface sous la nôtre : hors combat et chat verrouillé, miroir actif
-- (l'application dessine alors la bulle entière).
function B:Replaces()
  return self:Enabled() and CT.Mirror and CT.Mirror.mode == "on"
    and not (CT.InLockdown and CT.InLockdown()) and not (CT.InCombat and CT.InCombat())
end

-- Un jeu qui remet l'opacité de la bulle (apparition en fondu) la retrouve effacée aussitôt.
local hooked = setmetatable({}, { __mode = "k" })
local function Guard(visual)
  if hooked[visual] or not hooksecurefunc or not visual.SetAlpha then return end
  hooked[visual] = true
  pcall(hooksecurefunc, visual, "SetAlpha", function(self, a)
    if B.erased[self] and a ~= 0 then self:SetAlpha(0) end
  end)
end

-- Vrai si la bulle vient d'être effacée (elle ne l'était pas au relevé précédent).
local function Erase(visual)
  Guard(visual)
  if visual.GetAlpha and visual:GetAlpha() ~= 0 then visual:SetAlpha(0) end
  local new = not B.erased[visual]
  B.erased[visual] = true
  return new
end

-- Bulles de Blizzard rendues visibles, sauf celles de `keep` (encore remplacées).
local function Restore(keep)
  for visual in pairs(B.erased) do
    if not (keep and keep[visual]) then
      B.erased[visual] = nil
      pcall(visual.SetAlpha, visual, 1)
    end
  end
end
B.Restore = Restore

local function Overlaps(a, b)
  return a[1] < b[1] + b[3] and b[1] < a[1] + a[3] and a[2] < b[2] + b[4] and b[2] < a[2] + a[4]
end

local function U8(t, v) t[#t + 1] = floor(v) % 256 end
local function U16(t, v)
  v = floor(v) % 65536
  t[#t + 1] = floor(v / 256)
  t[#t + 1] = v % 256
end

-- Bulles visibles et traduisibles : { id, l, b, w, h, r, g, bl, px, font, text, visual }.
local function Scan()
  local list = {}
  if not (C_ChatBubbles and C_ChatBubbles.GetAllChatBubbles) then return list end
  local ok, bubbles = pcall(C_ChatBubbles.GetAllChatBubbles, false)
  if not ok or type(bubbles) ~= "table" then return list end
  local _, ph = GetPhysicalScreenSize()
  local k0 = (ph or 768) / 768
  local ui = CT.Mirror and CT.Mirror.UiRects and CT.Mirror:UiRects(k0) or {}
  for _, bubble in ipairs(bubbles) do
    if #list >= MAX then break end
    local okB, item = pcall(function()
      if bubble.IsForbidden and bubble:IsForbidden() then return nil end
      if not bubble:IsVisible() then return nil end
      local fs = FontStringOf(bubble)
      if not fs then return nil end
      local text = fs:GetText()
      if IsSecret(text) then return nil end
      local id, guid = B:Lookup(text)
      if not id then
        B.stats.unknown = B.stats.unknown + 1
        return nil
      end
      local l, b, w, h = fs:GetLeft(), fs:GetBottom(), fs:GetWidth(), fs:GetHeight()
      if IsSecret(l) or IsSecret(b) or IsSecret(w) or IsSecret(h) then return nil end
      if not (l and b and w and h) or w <= 0 or h <= 0 then return nil end
      local k = fs:GetEffectiveScale() * k0
      local rect = { floor(l * k), floor(b * k), floor(w * k + 0.5), floor(h * k + 0.5) }
      for _, r in ipairs(ui) do
        if Overlaps(rect, r) then
          B.stats.hidden = B.stats.hidden + 1
          return nil
        end
      end
      local path, size = fs:GetFont()
      local r, g, bl = fs:GetTextColor()
      if IsSecret(path) or IsSecret(size) or IsSecret(r) then return nil end
      local cr, cg, cb = ClassColor(guid)
      return { id = id, l = rect[1], b = rect[2], w = rect[3], h = rect[4],
        r = r or 1, g = g or 1, bl = bl or 1, px = (size or 12) * k, font = type(path) == "string" and path or "",
        text = Norm(text), visual = VisualOf(bubble),
        cr = cr, cg = cg, cb = cb, icon = SpecIcon(guid, GetTime()) }
    end)
    if not okB then
      B.stats.errors = B.stats.errors + 1
      B.lastError = tostring(item)
    elseif item then
      list[#list + 1] = item
    end
  end
  return list
end
B.Scan = Scan

-- Trame des bulles : [nombre] puis par bulle numéro, gauche, bas, largeur, hauteur (pixels
-- physiques depuis le bas à gauche), couleur (3 octets), taille de la police (dixièmes de pixel) ;
-- enfin la police (longueur, chemin).
local function Payload(list)
  local p = {}
  U8(p, #list)
  for _, e in ipairs(list) do
    U16(p, e.id)
    U16(p, max(0, e.l)); U16(p, max(0, e.b)); U16(p, e.w); U16(p, e.h)
    U8(p, min(255, e.r * 255 + 0.5)); U8(p, min(255, e.g * 255 + 0.5)); U8(p, min(255, e.bl * 255 + 0.5))
    U16(p, e.px * 10)
  end
  local path = (list[1] and list[1].font or ""):sub(1, 80)
  U8(p, #path)
  for c = 1, #path do p[#p + 1] = path:byte(c) end
  local room = (CT.MaxPayload and CT.MaxPayload() or 255) - #p
  -- Style de chaque bulle (0.16.4) : [64 + rang] [7] [couleur de classe : 3 octets] [icône de la
  -- spécialisation : numéro de fichier, 4 octets, 0 si inconnue]. Ignoré des versions d'avant.
  for i, e in ipairs(list) do
    if e.cr and room >= 9 then
      U8(p, 64 + i - 1)
      U8(p, 7)
      U8(p, min(255, e.cr * 255 + 0.5)); U8(p, min(255, e.cg * 255 + 0.5)); U8(p, min(255, e.cb * 255 + 0.5))
      local icon = e.icon or 0
      U16(p, floor(icon / 65536)); U16(p, icon % 65536)
      room = room - 9
    end
  end
  -- Textes des bulles (0.16.3), tant qu'il reste de la place : [rang] [longueur] [texte UTF-8].
  -- L'application s'en sert si elle a manqué le message (les anciennes versions l'ignorent).
  for i, e in ipairs(list) do
    local t = e.text or ""
    if #t > TEXT_MAX then
      t = t:sub(1, TEXT_MAX)
      -- Pas de caractère coupé en deux.
      while #t > 0 and t:byte(#t) >= 0x80 and t:byte(#t) < 0xC0 do t = t:sub(1, #t - 1) end
      if #t > 0 and t:byte(#t) >= 0xC0 then t = t:sub(1, #t - 1) end
    end
    if #t > 0 and #t + 2 <= room then
      U8(p, i - 1)
      U8(p, #t)
      for c = 1, #t do p[#p + 1] = t:byte(c) end
      room = room - #t - 2
    end
  end
  return p
end
B.Payload = Payload

function B:Poll(now)
  local list = {}
  if self:Enabled() and CT.Mirror and CT.Mirror.mode ~= "off" and not (CT.InLockdown and CT.InLockdown()) then
    list = Scan()
  end
  -- Bulles de Blizzard effacées sous les nôtres ; les autres (inconnues, sous une fenêtre du jeu,
  -- combat) rendues visibles.
  local keep
  if self:Replaces() then
    keep = {}
    for _, e in ipairs(list) do
      if e.visual then
        if Erase(e.visual) then self.stats.replaced = self.stats.replaced + 1 end
        keep[e.visual] = true
      end
    end
  end
  Restore(keep)
  local parts = {}
  for _, e in ipairs(list) do
    parts[#parts + 1] = format("%d:%d:%d:%d:%d:%s:%s", e.id, floor(e.l / MOVE), floor(e.b / MOVE), floor(e.w / MOVE), floor(e.h / MOVE),
      tostring(e.cr), tostring(e.icon))
  end
  local sig = table.concat(parts, ";")
  local due = sig ~= self.sentSig or (#list > 0 and now - self.sentAt >= REFRESH)
  if not due then return end
  self.sentSig, self.sentAt = sig, now
  if #list > 0 then self.stats.matched = self.stats.matched + #list end
  local p = Payload(list)
  -- Une seule trame de bulles en attente : la plus récente remplace l'ancienne.
  if CT.SetLatest then
    CT.SetLatest(CT.FRAME_BUBBLE, p, #p)
    self.stats.frames = self.stats.frames + 1
  end
end

function B:Command(arg)
  arg = (arg or ""):lower()
  if arg == "on" or arg == "off" then
    if arg == "off" then
      ChevaliersTranslatorsDB.bubbles = false
      self.sentSig = "?"
      Restore()
    else
      ChevaliersTranslatorsDB.bubbles = nil
    end
    CT.Print(arg == "on"
      and "Bulles traduites : la bulle de dialogue est remplacée par celle de l'application, traduite avec l'acronyme de la langue (hors donjons et raids, où le jeu les protège)."
      or "Bulles traduites : désactivé, les bulles restent telles quelles.")
  else
    CT.Print(format("Bulles traduites : %s - trames %d, bulles reconnues %d, remplacées %d, inconnues %d, sous une fenêtre %d%s",
      self:Enabled() and "actif" or "désactivé", self.stats.frames, self.stats.matched, self.stats.replaced, self.stats.unknown,
      self.stats.hidden, self.lastError and (", erreur : " .. self.lastError) or ""))
    CT.Print("/chevaliers bulles on | off")
  end
end

local driver = CreateFrame("Frame")
pcall(driver.RegisterEvent, driver, "INSPECT_READY")
driver:SetScript("OnEvent", function(_, event, guid)
  if event == "INSPECT_READY" then pcall(B.OnInspectReady, B, guid) end
end)
local acc = 0
driver:SetScript("OnUpdate", function(_, elapsed)
  acc = acc + (elapsed or 0)
  if acc < POLL then return end
  acc = 0
  local ok, err = pcall(B.Poll, B, GetTime())
  if not ok then
    B.stats.errors = B.stats.errors + 1
    B.lastError = tostring(err)
    -- Jamais de bulle de Blizzard laissée effacée après une erreur.
    pcall(Restore)
  end
end)
