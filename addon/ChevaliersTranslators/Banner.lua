-- Chevaliers Translators
-- Banner.lua : bandeau de pixels en bas à droite de l'écran (coin opposé à QuestVoice).
-- Le cadre n'a pas de parent : il échappe à l'échelle d'interface et à Alt+Z,
-- et son échelle est réglée pour qu'une unité vaille exactement un pixel physique.

local _, CT = ...

local floor = math.floor

local Banner = { block = 4, cols = 0, widthRatio = 0.25 }
CT.Banner = Banner

-- Niveaux de couleur (0.13.3) : bandeau sombre et discret ; l'application se cale sur les blocs
-- d'étalonnage de chaque trame, l'écart entre niveaux (38 sur 255) reste large pour la capture.
local LEVEL = { [0] = 0, 0.15, 0.30, 0.45 }
local frame
local textures = {}
-- Symbole affiché par chaque bloc (0.15) : seuls les blocs qui changent sont repeints. Une trame
-- courte (signe de vie, bulles) laisse la plupart des blocs à zéro : environ 10 fois moins de
-- textures modifiées par trame.
local painted = {}

local function PhysicalSize()
  if GetPhysicalScreenSize then
    local w, h = GetPhysicalScreenSize()
    if w and h and w > 0 and h > 0 then return w, h end
  end
  return nil
end

function Banner:Init(block)
  if block then self.block = block end
  if not frame then
    frame = CreateFrame("Frame", "ChevaliersTranslatorsBanner", nil)
    frame:SetFrameStrata("TOOLTIP")
    frame:SetFrameLevel(9000)
    frame:SetClampedToScreen(false)
    frame:EnableMouse(false)
    frame:Hide()
  end
  return self:Layout()
end

function Banner:Layout()
  if not frame then return false end
  local pw, ph = PhysicalSize()
  if not pw then
    self.error = "GetPhysicalScreenSize indisponible"
    return false
  end
  self.pw, self.ph = pw, ph

  -- Hauteur de l'écran en unités d'échelle 1, divisée par les pixels physiques.
  local h1 = UIParent:GetHeight() * UIParent:GetEffectiveScale()
  frame:SetScale(h1 / ph)

  local B = self.block
  local cols = floor(pw * self.widthRatio / B)
  if cols > 4095 then cols = 4095 end
  if cols < 20 then cols = 20 end
  self.cols = cols

  frame:ClearAllPoints()
  frame:SetPoint("BOTTOMRIGHT", nil, "BOTTOMRIGHT", 0, 0)
  frame:SetSize(cols * B, 2 * B)

  for i = 1, 2 * cols do
    local t = textures[i]
    if not t then
      t = frame:CreateTexture(nil, "OVERLAY")
      textures[i] = t
    end
    local row, k
    if i <= cols then
      row, k = 0, i - 1
    else
      row, k = 1, i - cols - 1
    end
    t:ClearAllPoints()
    t:SetSize(B, B)
    t:SetPoint("BOTTOMRIGHT", frame, "BOTTOMRIGHT", -k * B, row * B)
    t:SetColorTexture(0, 0, 0, 1)
    painted[i] = 0
    t:Show()
  end
  for i = 2 * cols + 1, #textures do textures[i]:Hide() end
  self.error = nil
  return true
end

function Banner:Paint(syms)
  for i = 1, 2 * self.cols do
    local s = syms[i]
    if painted[i] ~= s then
      painted[i] = s
      textures[i]:SetColorTexture(LEVEL[floor(s / 16)], LEVEL[floor(s / 4) % 4], LEVEL[s % 4], 1)
    end
  end
  if not frame:IsShown() then frame:Show() end
end

function Banner:Hide()
  if frame and frame:IsShown() then frame:Hide() end
end

function Banner:IsShown()
  return frame and frame:IsShown()
end
