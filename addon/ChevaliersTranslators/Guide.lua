-- Chevaliers Translators
-- Guide.lua : petit panneau « Guide d'installation » affiché une fois, à la première utilisation
-- de l'addon (et par /chevaliers guide), avec la bannière et le logo. Il dit où se trouve
-- l'application compagnon et comment la lancer. Fermeture par la croix ou au bout de 60 s.
-- Le panneau n'est construit qu'au premier affichage (rien à la connexion tant qu'il n'est pas
-- demandé). Appelé par Core.lua via CT.ShowGuide().

local _, CT = ...
CT = CT or {}

local MEDIA = "Interface\\AddOns\\ChevaliersTranslators\\Media\\"
local GUIDE_TEXT =
  "Bienvenue dans Chevaliers Translators !\n\n" ..
  "L'addon transmet le chat à l'application compagnon, qui le traduit sur ton PC (aucun serveur " ..
  "en ligne).\n\n" ..
  "|cffe6c77e1.|r L'application se trouve dans le dossier de l'addon : " ..
  "|cffffffffChevaliersTranslators.exe|r. Double-clique dessus pour la lancer ; elle pose ensuite " ..
  "un raccourci |cffffffffChevaliers Translators|r sur ton bureau et ouvre la fenêtre des modèles.\n" ..
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

-- Affiche le guide (et retient qu'il a été vu, pour ne plus l'ouvrir tout seul).
function CT.ShowGuide()
  if ChevaliersTranslatorsDB then ChevaliersTranslatorsDB.guideSeen = true end
  local ok, f = pcall(BuildGuide)
  if ok and f then f:Show() end
end
