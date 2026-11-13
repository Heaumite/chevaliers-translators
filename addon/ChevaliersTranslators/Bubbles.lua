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

local B = {
  recent = {},              -- texte normalisé -> { id, at }
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
function B:Note(id, event, text, skip)
  if skip or not BUBBLE_EVENTS[event] then return end
  local key = Norm(text)
  if not key then return end
  local now = GetTime()
  self.recent[key] = { id = id, at = now }
  -- Ménage de temps en temps.
  if (self.pruneAt or 0) < now - 10 then
    self.pruneAt = now
    for k, e in pairs(self.recent) do
      if now - e.at > KEEP then self.recent[k] = nil end
    end
  end
end

-- Message d'une bulle : son numéro de relais (texte identique, ou même début s'il est long).
function B:Lookup(text)
  local key = Norm(text)
  if not key then return nil end
  local e = self.recent[key]
  if e then return e.id end
  if #key >= 24 then
    local head = key:sub(1, 24)
    local best
    for k, v in pairs(self.recent) do
      if k:sub(1, 24) == head and (not best or v.at > best.at) then best = v end
    end
    return best and best.id
  end
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
      local id = B:Lookup(text)
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
      return { id = id, l = rect[1], b = rect[2], w = rect[3], h = rect[4],
        r = r or 1, g = g or 1, bl = bl or 1, px = (size or 12) * k, font = type(path) == "string" and path or "",
        text = Norm(text), visual = VisualOf(bubble) }
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
  -- Textes des bulles (0.16.3), tant qu'il reste de la place : [rang] [longueur] [texte UTF-8].
  -- L'application s'en sert si elle a manqué le message (les anciennes versions l'ignorent).
  local room = (CT.MaxPayload and CT.MaxPayload() or 255) - #p
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
    parts[#parts + 1] = format("%d:%d:%d:%d:%d", e.id, floor(e.l / MOVE), floor(e.b / MOVE), floor(e.w / MOVE), floor(e.h / MOVE))
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
