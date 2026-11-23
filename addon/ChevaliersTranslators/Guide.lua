-- Chevaliers Translators
-- Guide.lua : petit panneau de présentation affiché une fois, à la première utilisation
-- de l'addon et à chaque nouvelle révision de son contenu (et par /chevaliers guide).
-- 0.16.3 : bannière titrée « Chevaliers Translators », lien de la vidéo du guide d'installation et
-- lien du dépôt GitHub d'où viennent les mises à jour automatiques. WoW ne peut ni lire une vidéo
-- ni ouvrir un lien : chaque lien est dans un cadre où il se sélectionne d'un clic, Ctrl+C le
-- copie, il se colle dans le navigateur. Addons incompatibles chargés : signalés en rouge.
-- 0.16.5 : lien d'une musique de l'auteur, ce que fait l'addon une fois installé (toutes les
-- versions de WoW, toutes les langues européennes vers celle du joueur) et le dépôt GitHub de ses
-- mises à jour.
-- Fermeture par la croix, ou au bout de 60 s si aucun lien n'a été touché.
-- Le panneau n'est construit qu'au premier affichage. Appelé par Core.lua via CT.ShowGuide().

local _, CT = ...
CT = CT or {}

local MEDIA = "Interface\\AddOns\\ChevaliersTranslators\\Media\\"
-- Révision du contenu du guide : un guide changé s'affiche de nouveau une fois (0.15.1).
CT.GUIDE_REV = 4
CT.GUIDE_REPO = "https://github.com/Heaumite/chevaliers-translators"
CT.GUIDE_MUSIC = "https://www.youtube.com/watch?v=yvU-hxWE3VA"

local GOLD = { 0.902, 0.78, 0.494 }

-- Cadre d'un lien : texte sélectionné d'un clic, prêt pour Ctrl+C ; le lien ne se modifie pas.
local function LinkBox(f, url, y, width)
  local ok, eb = pcall(CreateFrame, "EditBox", nil, f, "InputBoxTemplate")
  if not ok or not eb then eb = CreateFrame("EditBox", nil, f) end
  pcall(eb.SetAutoFocus, eb, false)
  eb:SetSize(width or 446, 24)
  eb:SetPoint("TOPLEFT", f, "TOPLEFT", 30, y)
  pcall(eb.SetFontObject, eb, ChatFontNormal or GameFontHighlight)
  eb:SetText(url)
  pcall(eb.SetCursorPosition, eb, 0)
  local function selectAll(self)
    self:HighlightText()
    -- Lien touché : le panneau ne se ferme plus tout seul.
    f.hideAt = nil
  end
  eb:SetScript("OnEditFocusGained", selectAll)
  eb:SetScript("OnMouseUp", function(self) self:SetFocus() selectAll(self) end)
  eb:SetScript("OnTextChanged", function(self, user)
    if user then self:SetText(url) self:HighlightText() end
  end)
  eb:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)
  eb:SetScript("OnEnterPressed", function(self) self:ClearFocus() end)
  eb:SetScript("OnEditFocusLost", function(self) pcall(self.HighlightText, self, 0, 0) end)
  eb.url = url
  return eb
end

local function Label(f, template, text, y, color)
  local fs = f:CreateFontString(nil, "OVERLAY", template)
  fs:SetPoint("TOPLEFT", f, "TOPLEFT", 24, y)
  fs:SetPoint("RIGHT", f, "RIGHT", -24, 0)
  pcall(fs.SetJustifyH, fs, "LEFT")
  if color then fs:SetTextColor(color[1], color[2], color[3]) end
  fs:SetText(text)
  return fs
end

local guideFrame
local function BuildGuide()
  if guideFrame then return guideFrame end
  local f = CreateFrame("Frame", "ChevaliersTranslatorsGuide", UIParent)
  f:SetFrameStrata("DIALOG")
  f:SetSize(500, 332)
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
  -- Bannière de l'addon en haut, titrée dans son champ sombre (comme la fenêtre de l'application).
  local ban = f:CreateTexture(nil, "ARTWORK")
  ban:SetPoint("TOP", f, "TOP", 0, -8)
  ban:SetSize(468, 117)
  ban:SetTexture(MEDIA .. "Banniere")
  local title = f:CreateFontString(nil, "OVERLAY", "GameFontNormalHuge")
  pcall(title.SetFont, title, "Fonts\\FRIZQT__.TTF", 24, "")
  title:SetPoint("LEFT", ban, "LEFT", 150, 8)
  title:SetTextColor(GOLD[1], GOLD[2], GOLD[3])
  pcall(title.SetShadowOffset, title, 1, -1)
  pcall(title.SetShadowColor, title, 0, 0, 0, 1)
  title:SetText("Chevaliers Translators")
  local sub = f:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
  sub:SetPoint("TOPLEFT", title, "BOTTOMLEFT", 1, -5)
  sub:SetTextColor(0.81, 0.765, 0.635)
  pcall(sub.SetShadowOffset, sub, 1, -1)
  sub:SetText("Traduction du chat, sur ton PC")
  -- Croix de fermeture.
  local close = CreateFrame("Button", nil, f, "UIPanelCloseButton")
  close:SetPoint("TOPRIGHT", f, "TOPRIGHT", -2, -2)
  -- Musique de l'auteur.
  Label(f, "GameFontNormal", "Musique de l'auteur de l'addon :", -140, GOLD)
  f.music = LinkBox(f, CT.GUIDE_MUSIC, -162)
  -- Ce que fait l'addon une fois installé.
  local about = Label(f, "GameFontHighlight",
    "Une fois installé, l'addon fonctionne pour toutes les versions de World of Warcraft, dans toutes "
    .. "les langues européennes : il traduit les langues étrangères vers la langue d'origine du joueur, "
    .. "peu importe de quel pays il vient.", -196)
  pcall(about.SetSpacing, about, 3)
  f.about = about
  -- Mises à jour.
  Label(f, "GameFontNormal", "L'addon se met à jour automatiquement depuis :", -256, GOLD)
  f.repo = LinkBox(f, CT.GUIDE_REPO, -278)
  Label(f, "GameFontDisableSmall", "Clic dans un cadre, puis Ctrl+C pour copier le lien.", -306)
  -- Addons incompatibles chargés (rempli à chaque affichage).
  f.warn = Label(f, "GameFontHighlightSmall", "", -332, { 1, 0.35, 0.3 })
  pcall(f.warn.SetSpacing, f.warn, 2)
  -- Fermeture automatique au bout de 60 s après l'affichage (sauf lien touché).
  f:SetScript("OnShow", function(self) self.hideAt = GetTime() + 60 end)
  f:SetScript("OnHide", function(self)
    pcall(self.music.ClearFocus, self.music)
    pcall(self.repo.ClearFocus, self.repo)
  end)
  f:SetScript("OnUpdate", function(self)
    if self.hideAt and GetTime() >= self.hideAt then self.hideAt = nil self:Hide() end
  end)
  guideFrame = f
  return f
end

-- Addons incompatibles chargés : une ligne par addon dans le guide, panneau agrandi d'autant.
local function FillWarnings(f)
  local ok, bad = pcall(function() return CT.IncompatibleAddons and CT.IncompatibleAddons() or {} end)
  local lines = {}
  for _, a in ipairs(ok and bad or {}) do
    lines[#lines + 1] = "Addon incompatible : " .. a.title .. ". " .. a.why
  end
  f.warn:SetText(table.concat(lines, "\n"))
  f:SetHeight(#lines > 0 and 348 + 40 * #lines or 332)
  f.warnings = #lines
end

-- Notification de l'addon (0.16.6) : petit panneau en haut de l'écran, logo, titre et texte,
-- effacé en fondu au bout de `seconds` (clic : fermé tout de suite).
local toast
function CT.Toast(title, text, seconds)
  local ok = pcall(function()
    if not toast then
      local f = CreateFrame("Frame", "ChevaliersTranslatorsToast", UIParent)
      f:SetFrameStrata("DIALOG")
      f:SetSize(460, 64)
      f:SetPoint("TOP", UIParent, "TOP", 0, -150)
      f:EnableMouse(true)
      local bg = f:CreateTexture(nil, "BACKGROUND")
      bg:SetAllPoints(f)
      bg:SetColorTexture(0.055, 0.051, 0.039, 0.92)
      local edge = f:CreateTexture(nil, "BORDER")
      edge:SetPoint("TOPLEFT")
      edge:SetPoint("BOTTOMLEFT")
      edge:SetWidth(3)
      edge:SetColorTexture(0.788, 0.647, 0.353, 1)
      local logo = f:CreateTexture(nil, "ARTWORK")
      logo:SetSize(40, 40)
      logo:SetPoint("LEFT", f, "LEFT", 12, 0)
      logo:SetTexture(MEDIA .. "Logo")
      f.title = f:CreateFontString(nil, "OVERLAY", "GameFontNormal")
      f.title:SetPoint("TOPLEFT", f, "TOPLEFT", 62, -10)
      f.title:SetPoint("RIGHT", f, "RIGHT", -12, 0)
      pcall(f.title.SetJustifyH, f.title, "LEFT")
      f.text = f:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
      f.text:SetPoint("TOPLEFT", f.title, "BOTTOMLEFT", 0, -4)
      f.text:SetPoint("RIGHT", f, "RIGHT", -12, 0)
      pcall(f.text.SetJustifyH, f.text, "LEFT")
      f:SetScript("OnMouseUp", function(self) self:Hide() end)
      f:SetScript("OnUpdate", function(self)
        local left = (self.hideAt or 0) - GetTime()
        if left <= 0 then self:Hide() return end
        pcall(self.SetAlpha, self, math.min(1, left / 1.5))
      end)
      toast = f
    end
    toast.title:SetText(title)
    toast.text:SetText(text)
    local h = 26 + (toast.text.GetStringHeight and toast.text:GetStringHeight() or 24)
    toast:SetHeight(math.max(64, h + 14))
    toast.hideAt = GetTime() + (seconds or 10)
    pcall(toast.SetAlpha, toast, 1)
    toast:Show()
  end)
  return ok and toast
end

-- Affiche le guide (et retient qu'il a été vu, pour ne plus l'ouvrir tout seul).
function CT.ShowGuide()
  if ChevaliersTranslatorsDB then ChevaliersTranslatorsDB.guideSeen = CT.GUIDE_REV end
  local ok, f = pcall(BuildGuide)
  if ok and f then
    pcall(FillWarnings, f)
    f:Show()
  end
end
