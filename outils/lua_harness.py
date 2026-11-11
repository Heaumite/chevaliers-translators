"""Harnais de test hors jeu de l'addon ChevaliersTranslators.

1. vectors : exécute Encoder.lua sous Lua 5.1 (bit pur Lua, non signé) et sous LuaJIT
   (bit natif, signé), vérifie que les deux donnent les mêmes trames, puis écrit
   app/tests/lua_vectors.txt que le test Rust compare à son propre encodeur.
2. mock    : charge Encoder.lua, Banner.lua, Core.lua et Mirror.lua dans une imitation de
   l'API WoW, exécute les commandes /chevaliers, rend chaque trame peinte en image BGRA
   et la fait décoder par `ct-outils decode-raw`.

Usage : python3 outils/lua_harness.py vectors|mock <dossier du dépôt> [binaire ct-outils]
"""
import os
import shutil
import subprocess
import sys
import tempfile

from lupa import lua51, luajit21

PURE_BIT = r"""
local M = 4294967296
local floor = math.floor
local function norm(x) return x % M end
local function bitop(a, b, f)
  a, b = norm(a), norm(b)
  local r, p = 0, 1
  for _ = 1, 32 do
    local x, y = a % 2, b % 2
    if f(x, y) then r = r + p end
    a, b, p = (a - x) / 2, (b - y) / 2, p * 2
  end
  return r
end
bit = {
  band = function(a, b) return bitop(a, b, function(x, y) return x == 1 and y == 1 end) end,
  bor = function(a, b) return bitop(a, b, function(x, y) return x == 1 or y == 1 end) end,
  bxor = function(a, b) return bitop(a, b, function(x, y) return x ~= y end) end,
  rshift = function(a, n) return floor(norm(a) / 2 ^ n) end,
  lshift = function(a, n) return norm(norm(a) * 2 ^ n) end,
}
"""

CHAT_TEXT = ("Guten Abend zusammen ! Wer kommt mit in den Schwarzfels ? Ça fait plaisir, ŝ€ ✓ Привет всем "
             "|cffffd000|Hitem:19019::::::::60:::::|h[Donnerzorn]|h|r et encore un peu de texte pour dépasser "
             "la taille d'un fragment, encore et encore, jusqu'à la fin.")

VECTOR_GEN = r"""
local CT, chat = ...
local out = {}
local function hex(t, n)
  local s = {}
  for i = 1, n do s[i] = string.format("%02x", t[i]) end
  return table.concat(s)
end
for _, cols in ipairs({ 40, 87, 120, 160, 240 }) do
  local _, _, maxp = CT.Capacity(cols)
  for _, seq in ipairs({ 0, 1, 2, 37, 255, 256, 1000, 4097, 65535 }) do
    local p, syms = {}, {}
    local n = CT.TestPayload(seq, maxp, p)
    assert(CT.BuildBanner(CT.FRAME_TEST, seq, p, n, cols, syms))
    out[#out + 1] = string.format("T %d %d %s", cols, seq, hex(syms, 2 * cols))
  end
  local frags = {}
  CT.ChatFragments(4242, 10, "Hans-Ysondre", chat, maxp, function(p, len)
    local copy = {}
    for i = 1, len do copy[i] = p[i] end
    frags[#frags + 1] = { p = copy, n = len }
  end)
  for i, f in ipairs(frags) do
    local syms = {}
    assert(CT.BuildBanner(CT.FRAME_CHAT, 4242, f.p, f.n, cols, syms))
    out[#out + 1] = string.format("C %d %d %s %d", cols, 4242, hex(syms, 2 * cols), i)
  end
end
return table.concat(out, "\n")
"""


def load_encoder(lua, root, pure_bit):
    if pure_bit:
        lua.execute(PURE_BIT)
    src = open(os.path.join(root, "addon/ChevaliersTranslators/Encoder.lua"), encoding="utf-8").read()
    chunk = lua.eval("function(s) return assert(loadstring(s, 'Encoder.lua')) end")(src)
    ct = lua.table()
    chunk("ChevaliersTranslators", ct)
    return ct


def lint_secret_order(root):
    """Une valeur peut-être secrète (WoW 12) ne doit être ni testée ni comparée avant le contrôle
    Secret/IsSecret/issecretvalue : `x and not Secret(x)` lève « attempt to perform boolean test on
    a secret value » (0.14.3, UnitIsAFK en champ de bataille)."""
    import re
    bad = []
    pats = [
        re.compile(r"\b(\w+(?:\.\w+)?)\s+and\s+not\s+(?:Secret|IsSecret)\(\1\)"),
        re.compile(r"\b(\w+(?:\.\w+)?)\s*==\s*[^=\n]+?\s+and\s+not\s+(?:Secret|IsSecret)\(\1\)"),
        re.compile(r"not\s+\(([\w ]+?(?:\band\b[\w ]+)+)\)\s+or\s+(?:Secret|IsSecret)\("),
    ]
    for name in ("Core.lua", "Mirror.lua", "Banner.lua", "Encoder.lua"):
        path = os.path.join(root, "addon", "ChevaliersTranslators", name)
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                if any(p.search(line) for p in pats):
                    bad.append(f"{name}:{n}: {line.strip()}")
    if bad:
        sys.exit("ÉCHEC : valeur testée avant le contrôle des valeurs secrètes :\n  " + "\n  ".join(bad))
    print("Valeurs secrètes : contrôlées avant tout test ou comparaison")


def vectors(root):
    results = []
    for name, runtime, pure in (("Lua 5.1", lua51, True), ("LuaJIT 2.1", luajit21, False)):
        lua = runtime.LuaRuntime(unpack_returned_tuples=True)
        ct = load_encoder(lua, root, pure)
        gen = lua.eval("function(s) return assert(loadstring(s)) end")(VECTOR_GEN)
        text = gen(ct, CHAT_TEXT)
        results.append((name, text))
        print(f"{name} : {len(text.splitlines())} vecteurs")
    if results[0][1] != results[1][1]:
        sys.exit("ÉCHEC : Lua 5.1 et LuaJIT produisent des trames différentes")
    path = os.path.join(root, "app/tests/lua_vectors.txt")
    with open(path, "w", encoding="utf-8") as f:
        f.write("# Généré par outils/lua_harness.py à partir de Encoder.lua (Lua 5.1 et LuaJIT identiques)\n")
        f.write(results[0][1] + "\n")
    print(f"Lua 5.1 et LuaJIT identiques -> {path}")


WOW_MOCK = r"""
local ROOT = ...
NOW = 1000
LOG, SENT = {}, {}
SECRET = setmetatable({}, { __tostring = function() return "<secret>" end })
local function noop() end

local Texture = {}
Texture.__index = Texture
function Texture:SetSize(w, h) self.w, self.h = w, h end
function Texture:ClearAllPoints() self.ox, self.oy = nil, nil end
function Texture:SetPoint(pt, rel, relPt, x, y) self.pt, self.ox, self.oy = pt, x or 0, y or 0 end
function Texture:SetColorTexture(r, g, b, a) self.r, self.g, self.b = r, g, b end
function Texture:Show() self.shown = true end
function Texture:Hide() self.shown = false end
function Texture:SetAllPoints() end

local Frame = {}
Frame.__index = Frame
FRAMES = {}
function CreateFrame(kind, name, parent)
  local f = setmetatable({ scripts = {}, events = {}, textures = {}, shown = true, scale = 1, name = name, parent = parent }, Frame)
  FRAMES[#FRAMES + 1] = f
  if name then _G[name] = f end
  return f
end
function Frame:SetScript(k, fn) self.scripts[k] = fn end
function Frame:RegisterEvent(e) self.events[e] = true end
Frame.SetFrameStrata, Frame.SetFrameLevel, Frame.SetClampedToScreen, Frame.EnableMouse = noop, noop, noop, noop
Frame.ClearAllPoints, Frame.SetPoint = noop, noop
function Frame:SetSize(w, h) self.w, self.h = w, h end
function Frame:SetScale(s) self.scale = s end
function Frame:Hide() self.shown = false end
function Frame:Show() self.shown = true end
function Frame:IsShown() return self.shown end
function Frame:CreateTexture() local t = setmetatable({ shown = true }, Texture); self.textures[#self.textures + 1] = t; return t end
function Frame:SetAlpha(a) self.alpha = a end
function Frame:GetParent() return self.parent end
function Frame:GetFrameLevel() return 1 end
function Frame:SetFontString(fs) self.fs = fs end
function Frame:RegisterForClicks() end
function Frame:SetText(t) self.text = t end
function Frame:SetFocus() self.focus = true end
function Frame:HookScript(k, fn) self.hooks = self.hooks or {}; self.hooks[k] = fn end
-- Texte de mesure : largeur fixe par caractère (I 3, W 10, espace 4, autres 7), sans crénage.
local Probe = {}
Probe.__index = Probe
local CHAR_W = { I = 3, W = 10, [" "] = 4 }
function Probe:SetPoint() end
function Probe:SetFont(path, size, flags) self.font = path; return true end
function Probe:SetText(t) self.text = t end
function Probe:SetWidth(w) self.width = w end
function Probe:SetHeight(h) self.height = h end
function Probe:SetIndentedWordWrap(v) self.indented = v end
function Probe:GetUnboundedStringWidth()
  local w = 0
  for ch in (self.text or ""):gsub("||", "|"):gmatch("[%z\1-\127\194-\244][\128-\191]*") do
    w = w + (CHAR_W[ch] or 7)
  end
  return w
end
function Probe:CalculateScreenAreaFromCharacterSpan(a, b)
  -- Un mot par ligne ; lignes suivantes en retrait de 12 unités.
  return { { left = 0, bottom = 24, width = 30, height = 12 }, { left = 12, bottom = 12, width = 30, height = 12 },
           { left = 12, bottom = 0, width = 30, height = 12 } }
end
function Frame:CreateFontString() return setmetatable({}, Probe) end

UIParent = { GetHeight = function() return 1200 end, GetEffectiveScale = function() return 0.64 end }
function GetPhysicalScreenSize() return 1920, 1080 end
function GetTime() return NOW end
function debugprofilestop() return NOW * 1000 end
function GetBuildInfo() return "1.60.1", "70790", "Nov 2026", 16001 end
WOW_PROJECT_ID = 99
-- Fenêtres de chat : 1 générale, 2 journal de combat, 3 masquée sans GetFont.
-- Imitation de ScrollingMessageFrameMixin : historique, lignes visibles (messageInfo),
-- conteneur des lignes dont l'addon règle l'opacité.
local function FontStr(lineH)
  local alpha = 1
  return {
    GetAlpha = function() return alpha end,
    SetAlpha = function(_, a) alpha = a end,
    GetFont = function() return "Fonts\\ARIALN.TTF", 14, "" end,
    GetLineHeight = function() return lineH end,
    GetSpacing = function() return 0 end,
    GetShadowOffset = function() return 1, -1 end,
    GetShadowColor = function() return 0, 0, 0, 1 end,
    -- Ligne coupée si le message est long ; lignes suivantes en retrait de 12 unités.
    GetNumLines = function(self)
      local m = self.messageInfo and self.messageInfo.message
      return (type(m) == "string" and #m > 60) and 2 or 1
    end,
    CalculateScreenAreaFromCharacterSpan = function(self, a, b)
      return { { left = 0, bottom = 12, width = 400, height = 12 }, { left = 12, bottom = 0, width = 100, height = 12 } }
    end,
  }
end
local function ChatWindow(shown)
  local cf = { history = {}, visibleLines = {}, scrollOffset = 0, overrideFadeTimestamp = 0, shown = shown }
  local alpha = 1
  cf.FontStringContainer = { GetAlpha = function() return alpha end, SetAlpha = function(_, a) alpha = a end }
  for i = 1, 9 do cf.visibleLines[i] = FontStr(15.5) end
  function cf:Refresh()
    for i, fs in ipairs(self.visibleLines) do
      fs.messageInfo = self.history[#self.history - (i - 1) - self.scrollOffset]
    end
  end
  -- Comme ScrollingMessageFrame : l'affichage remet chaque ligne à l'opacité de WoW.
  function cf:RefreshDisplay()
    self:Refresh()
    for _, fs in ipairs(self.visibleLines) do fs:SetAlpha(1) end
  end
  function cf:UpdateFading() end
  function cf:CalculateLineAlphaValueFromTimestamp(now, ts) return 1 end
  -- Comme PackageEntry : données en plus (numéro du type, accès, événement...) gardées telles quelles.
  function cf:AddMessage(m, r, g, b, ...)
    if not issecretvalue(m) then LOG[#LOG + 1] = m end
    local n = select("#", ...)
    self.history[#self.history + 1] = { message = m, r = r, g = g, b = b, timestamp = NOW,
      extraData = n > 0 and { n = n, ... } or nil }
    self:RefreshDisplay()
  end
  function cf:IsShown() return self.shown end
  cf.IsVisible = cf.IsShown
  cf.GetFont = function() return "Fonts\\ARIALN.TTF", 14, "" end
  cf.GetShadowOffset = function() return 1, -1 end
  cf.GetEffectiveScale = function() return 0.64 end
  cf.GetLeft = function() return 32 end
  cf.GetBottom = function() return 95 end
  cf.GetWidth = function() return 430 end
  cf.GetHeight = function() return 120.5 end
  cf.GetTextColor = function() return 1, 1, 1 end
  cf.CanEffectivelyFade = function(self) return self.scrollOffset == 0 end
  cf.GetTimeVisible = function() return 120 end
  cf.GetFadeDuration = function() return 3 end
  return cf
end
-- Opacité de la ligne secrète (nil : aucune à l'écran).
function SECRET_ALPHA()
  for _, fs in ipairs(ChatFrame1.visibleLines) do
    if fs.messageInfo and fs.messageInfo.message == SECRET then return fs:GetAlpha() end
  end
end
-- Texte officiel des lignes de joueurs : 0 si l'addon les cache (l'application les dessine).
function OFFICIAL_ALPHA()
  for _, fs in ipairs(ChatFrame1.visibleLines) do
    if fs.messageInfo and fs.messageInfo.extraData and fs:GetAlpha() == 0 then return 0 end
  end
  return ChatFrame1.FontStringContainer:GetAlpha()
end
-- Numéros des types de message, comme ChatTypeInfo[type].id : sur retail 12.1.5 ils ne
-- distinguent pas les types (même numéro ici) ; seul l'événement rangé avec la ligne compte.
ChatTypeInfo = { SAY = { id = 1 }, LOOT = { id = 1 }, SYSTEM = { id = 1 }, CHANNEL2 = { id = 1 } }
function hooksecurefunc(t, name, fn)
  local orig = t[name]
  t[name] = function(...) local r = { orig(...) }; fn(...); return unpack(r) end
end
-- Ligne de chat d'un type donné, comme la fenêtre de chat de WoW : messages des joueurs et des
-- créatures avec leur événement (4e donnée), messages du jeu avec le numéro du type seul.
local GAME = { LOOT = true, SYSTEM = true }
function ADD(text, kind, r, g, b)
  local id = ChatTypeInfo[kind] and ChatTypeInfo[kind].id
  if GAME[kind] then
    ChatFrame1:AddMessage(text, r or 1, g or 1, b or 1, id)
  else
    local event = "CHAT_MSG_" .. (kind:match("^CHANNEL") and "CHANNEL" or kind)
    ChatFrame1:AddMessage(text, r or 1, g or 1, b or 1, id, 0, 0, event, {}, nil)
  end
end
LOCKDOWN = false
ChatFrame1, ChatFrame2 = ChatWindow(true), ChatWindow(true)
ChatFrame3 = { IsShown = function() return false end }
DEFAULT_CHAT_FRAME = ChatFrame1
ChatFrame1EditBox = CreateFrame("EditBox", "ChatFrame1EditBox")
ChatFontNormal = { GetFont = function() return "Fonts\\ARIALN.TTF", 14, "" end, GetShadowOffset = function() return 1, -1 end }
function IsCombatLog(f) return f == ChatFrame2 end
function GetChatWindowInfo(i)
  local names = { "Général", "Journal de combat", "Commerce" }
  return names[i], 14, 1, 1, 1, 1, i <= 2, false, i == 2 and 1 or nil
end
function GetLocale() return "frFR" end
function GetFramerate() return 44.04 end
SlashCmdList = {}
SECRETS = {}
function issecretvalue(v) return v == SECRET or (v ~= nil and SECRETS[v] == true) end
function UnitName() return "Testeur" end -- Forever : prénom seul
function GetUnitName() return "Testeur Batonnoir" end
function UnitFullName() return "Testeur Batonnoir", nil end
function GetNumLanguages() return 2 end
function GetLanguageByIndex(i) if i == 1 then return "Commun", 7 else return "Darnassien", 2 end end
SHIFT = false
function IsShiftKeyDown() return SHIFT end
IN_COMBAT = false
function InCombatLockdown() return IN_COMBAT end
function IsInInstance() return false, "none" end
function CT_HELD() return CT_TABLE.HeldStats end
BINDINGS = {}
function SetOverrideBinding(owner, prio, key, cmd) BINDINGS[key] = cmd end
ChatFrameEditBoxMixin = { OnPreSendText = function() end }
CALLBACKS = {}
EventRegistry = { RegisterCallback = function(self, event, fn, owner) CALLBACKS[event] = { fn = fn, owner = owner } end }
BLIZZ_SENT = {}
-- Imitation de ChatFrameEditBoxMixin:SendMessage : rappel, lecture du texte et envoi, puis
-- ClearChat (retour au type « collant », boîte vidée et fermée).
function SIMULATE_SEND(eb)
  local cb = CALLBACKS["ChatFrame.OnEditBoxPreSendText"]
  if cb then cb.fn(cb.owner, eb) end
  local text = eb:GetText()
  if text:find("%S") then BLIZZ_SENT[#BLIZZ_SENT + 1] = eb.attrs.chatType .. ":" .. tostring(eb.attrs.tellTarget) .. ":" .. text end
  if eb.attrs.chatType == "WHISPER" or eb.attrs.chatType == "YELL" then eb.attrs.chatType = "SAY" end
  eb.text = ""
  eb.shown = false
end
function EDITBOX(text, chatType, target)
  local eb = { text = text, attrs = { chatType = chatType, tellTarget = target }, languageID = 7, shown = true }
  function eb:GetText() return self.text end
  function eb:SetText(t) self.text = t end
  function eb:GetAttribute(k) return self.attrs[k] end
  function eb:SetAttribute(k, v) self.attrs[k] = v end
  function eb:Show() self.shown = true end
  function eb:Hide() self.shown = false end
  function eb:IsShown() return self.shown end
  function eb:SetFocus() self.focus = true end
  return eb
end
function time() return os.time() end
-- Bulle d'aide posée sur le chat (rectangle en unités de l'interface, échelle 0,64).
GameTooltip = { visible = false }
function GameTooltip:IsVisible() return self.visible end
function GameTooltip:GetLeft() return 100 end
function GameTooltip:GetBottom() return 150 end
function GameTooltip:GetWidth() return 200 end
function GameTooltip:GetHeight() return 100 end
function GameTooltip:GetEffectiveScale() return 0.64 end
function UnitGUID() return "Player-1-0ABC" end
-- Menus récents (MenuUtil) : le contenu du menu est relevé dans MENU.
local function MenuRoot(list)
  local r = {}
  function r:CreateTitle(t) list[#list + 1] = { title = t } end
  function r:CreateDivider() list[#list + 1] = { divider = true } end
  function r:CreateRadio(label, isSel, setSel, data)
    list[#list + 1] = { radio = label, sel = function() return isSel(data) end, set = function() setSel(data) end }
  end
  function r:CreateButton(label)
    local sub = {}
    list[#list + 1] = { button = label, sub = sub }
    return MenuRoot(sub)
  end
  return r
end
function SETUP_MENU()
  MenuUtil = { CreateContextMenu = function(owner, gen) MENU = {}; gen(owner, MenuRoot(MENU)) end }
end
function GetDefaultLanguage() return "Commun", 7 end

C_AddOns = {
  LoadAddOn = function(name) return false, "MISSING" end,
  IsAddOnLoaded = function(name) return false end,
  GetNumAddOns = function() return 1 end,
  GetAddOnInfo = function() return "QuestVoice" end,
}
C_ChatInfo = {
  InChatMessagingLockdown = function() return LOCKDOWN end,
  SendChatMessage = function(text, kind, lang, target) SENT[#SENT + 1] = kind .. ":" .. tostring(target) .. ":" .. tostring(lang) .. ":" .. text end,
}

local CT = {}
CT_TABLE = CT
for _, file in ipairs({ "Encoder.lua", "Banner.lua", "Core.lua", "Mirror.lua" }) do
  local f = assert(loadfile(ROOT .. "/addon/ChevaliersTranslators/" .. file))
  f("ChevaliersTranslators", CT)
end

function FIRE(event, ...)
  for _, f in ipairs(FRAMES) do
    if f.events[event] and f.scripts.OnEvent then f.scripts.OnEvent(f, event, ...) end
  end
end

-- Avance le temps d'une image et exécute les OnUpdate ; renvoie true si le bandeau a changé.
function TICK(dt)
  NOW = NOW + dt
  local banner = ChevaliersTranslatorsBanner
  local before = banner and banner.paints or 0
  for _, f in ipairs(FRAMES) do
    if f.scripts.OnUpdate then f.scripts.OnUpdate(f, dt) end
  end
  return banner
end

-- Intercepte Paint pour compter les trames peintes.
local paint = CT.Banner.Paint
CT.Banner.Paint = function(self, syms)
  paint(self, syms)
  ChevaliersTranslatorsBanner.paints = (ChevaliersTranslatorsBanner.paints or 0) + 1
end

function SLASH(msg) SlashCmdList.CHEVALIERS(msg) end
return CT
"""


def render_banner(lua_frame, w, h, ph=1080, virt=768.0):
    """Rend les textures du bandeau (coin bas-droit) en BGRA, comme à l'écran."""
    import random
    rnd = random.Random(7)
    img = bytearray(w * h * 4)
    for i in range(0, len(img), 4):
        img[i:i + 4] = bytes((rnd.randrange(256), rnd.randrange(256), rnd.randrange(256), 255))
    px = lua_frame.scale * ph / virt  # pixels physiques par unité du cadre
    for t in lua_frame.textures.values():
        if not t.shown:
            continue
        right = (w - 1) + round(t.ox * px)
        bottom = (h - 1) - round(t.oy * px)
        tw, th = round(t.w * px), round(t.h * px)
        color = (round(t.b * 255), round(t.g * 255), round(t.r * 255))
        for y in range(bottom - th + 1, bottom + 1):
            for x in range(right - tw + 1, right + 1):
                o = (y * w + x) * 4
                img[o:o + 3] = bytes(color)
    return bytes(img)


def mock(root, binary):
    work = tempfile.mkdtemp(prefix="ct-mock-")
    wow = os.path.join(work, "World of Warcraft")
    os.makedirs(os.path.join(wow, "_forever_"))
    subprocess.run([binary, "install", "--wow", wow, "--flavor", "_forever_", "--interface", "16001"], check=True)
    addons = os.path.join(wow, "_forever_", "Interface", "AddOns")
    for name in ("Encoder.lua", "Banner.lua", "Core.lua", "Mirror.lua"):
        installed = open(os.path.join(addons, "ChevaliersTranslators", name), encoding="utf-8").read()
        source = open(os.path.join(root, "addon/ChevaliersTranslators", name), encoding="utf-8").read()
        assert installed == source, f"{name} installé différent de la source"
    toc = open(os.path.join(addons, "ChevaliersTranslators", "ChevaliersTranslators.toc"), encoding="utf-8").read()
    assert "## Title: Chevaliers Translators\n" in toc and "SavedVariables: ChevaliersTranslatorsDB" in toc, toc
    assert not os.path.exists(os.path.join(addons, "ChevaliersTranslators", "Bindings.xml")), "raccourci de la phase 0"
    assert "## IconTexture: Interface\\AddOns\\ChevaliersTranslators\\Media\\Logo\n" in toc, toc
    logo = open(os.path.join(addons, "ChevaliersTranslators", "Media", "Logo.tga"), "rb").read()
    assert logo == open(os.path.join(root, "addon/ChevaliersTranslators/Media/Logo.tga"), "rb").read(), "logo installé"

    lua = lua51.LuaRuntime(unpack_returned_tuples=True)
    lua.execute(PURE_BIT)
    env = lua.eval("function(s) return assert(loadstring(s, 'mock')) end")(WOW_MOCK)
    ct = env(root)
    g = lua.globals()
    g.FIRE("PLAYER_LOGIN")
    banner = g.ChevaliersTranslatorsBanner
    print("Journal :", list(g.LOG.values()))
    assert banner.scale > 0, "échelle du bandeau non réglée"
    # Application absente à la connexion : l'addon attend (signe toutes les 10 s) ; coupé pour les
    # tests de trames, essayé plus bas.
    assert ct.waiting is True, "attente de l'application non notée"
    ct.waiting = None

    w, h = int(1920 * 0.30) + 8, 48
    decoded = 0

    def pump(max_ticks, expect):
        nonlocal decoded
        seen = 0
        for _ in range(max_ticks):
            before = banner.paints or 0
            g.TICK(1 / 60)
            if (banner.paints or 0) != before and banner.shown:
                raw = os.path.join(work, f"frame{decoded}.bgra")
                with open(raw, "wb") as f:
                    f.write(render_banner(banner, w, h))
                r = subprocess.run([binary, "decode-raw", raw, str(w), str(h)], capture_output=True, text=True)
                print("  decode-raw :", (r.stdout + r.stderr).strip().replace("\n", " | "))
                if r.returncode != 0:
                    sys.exit("ÉCHEC : trame de l'addon non décodée")
                decoded += 1
                seen += 1
                if seen >= expect:
                    return

    g.SLASH("test 4 20")
    pump(200, 4)
    g.SLASH("ping Bonjour depuis le harnais")
    pump(60, 1)
    g.SLASH("chat on")
    g.FIRE("CHAT_MSG_GUILD", "Guten Abend zusammen, wer kommt mit nach Schwarzfels ?", "Hans-Ysondre")
    pump(60, 1)
    queued_before = banner.paints
    g.FIRE("CHAT_MSG_WHISPER", g.SECRET, "Hans-Ysondre")
    # Langue raciale inconnue du personnage (charabia du jeu) : rien n'est transmis.
    g.FIRE("CHAT_MSG_SAY", "revas il rath'is", "Curttini-Hyjal", "Orc")
    for _ in range(30):
        g.TICK(1 / 60)
    assert banner.paints == queued_before, "un message secret a été transmis"
    # Nom complet appris depuis un /dire du joueur (12e argument = GUID du joueur).
    g.FIRE("CHAT_MSG_SAY", "test", "Testeur Batonnoir", "Commun", "", "Testeur Batonnoir", "", 0, 0, "", 7, 1, "Player-1-0ABC")
    # ---- Bouton Trad (0.9) ------------------------------------------------------
    tb = g.ChatFrame1EditBox.ctTrad
    assert tb is not None and tb.text == "Trad:AUTO", "bouton Trad absent"
    # Client sans menus : le clic passe à la langue suivante.
    tb.scripts["OnClick"](tb, "LeftButton")
    assert tb.text == "Trad:FR", f"bouton Trad : {tb.text}"
    tb.scripts["OnClick"](tb, "RightButton")
    assert tb.text == "Trad:AUTO" and g.ChevaliersTranslatorsDB.dest == "AUTO", "clic droit : retour à AUTO"
    # Client récent : menu des langues à un seul niveau (Automatique en tête, puis par région).
    g.SETUP_MENU()
    tb.scripts["OnClick"](tb, "LeftButton")
    menu = g.MENU
    titles = [e.title for e in menu.values() if e.title]
    radios = [e.radio for e in menu.values() if e.radio]
    subs = [e.button for e in menu.values() if e.button]
    print("Menu Trad :", titles, len(radios), "langues")
    assert titles[0] == "Langue du message envoyé", titles
    assert radios[:7] == ["Automatique", "Français", "Anglais", "Allemand", "Espagnol", "Italien", "Russe"], radios
    assert len(radios) == 24 and "Suédois" in radios and "Grec" in radios, radios
    assert "Europe du Nord" in titles and not subs, "sous-menus : un clic de trop"
    auto = [e for e in menu.values() if e.radio == "Automatique"][0]
    assert auto.sel() is True, "Automatique sélectionné par défaut"
    [e for e in menu.values() if e.radio == "Allemand"][0].set()
    assert tb.text == "Trad:DE", tb.text
    tb.scripts["OnClick"](tb, "RightButton")
    g.MenuUtil = None
    print("Bouton Trad : AUTO -> FR -> AUTO ; menu : Allemand choisi")

    # ---- S3 : Maj+Entrée, collage + Entrée simulés (0.11) ----------------------------------
    g.SLASH("lang de")
    assert tb.text == "Trad:DE", "choix par commande non reflété sur le bouton"
    assert "SHIFT-ENTER" not in g.BINDINGS, "raccourci Maj+Entrée boîte fermée encore posé"
    assert g.CALLBACKS["ChatFrame.OnEditBoxPreSendText"] is not None, "rappel avant envoi non enregistré"
    eb = g.EDITBOX("Bonjour normal", "SAY", None)
    g.SHIFT = False
    g.SIMULATE_SEND(eb)
    assert list(g.BLIZZ_SENT.values()) == ["SAY:nil:Bonjour normal"], "Entrée a été modifiée"
    eb = g.EDITBOX("/dance", "SAY", None)
    g.SHIFT = True
    g.SIMULATE_SEND(eb)
    assert len(g.BLIZZ_SENT) == 2 and g.BLIZZ_SENT[2] == "SAY:nil:/dance", "une commande a été interceptée"

    def frames_until_idle(tag, ticks=90):
        out = []
        for _ in range(ticks):
            before = banner.paints or 0
            g.TICK(1 / 60)
            if (banner.paints or 0) != before and banner.shown:
                raw = os.path.join(work, f"{tag}_{len(out)}.bgra")
                with open(raw, "wb") as f:
                    f.write(render_banner(banner, w, h))
                out.append(raw)
        return out

    s3_frames = []

    def app(frames):
        # L'application garde son état d'un envoi à l'autre : toutes les trames depuis le début.
        s3_frames.extend(frames)
        r = subprocess.run([binary, "feed", "--addons", addons, "--w", str(w), "--h", str(h)] + s3_frames,
                           capture_output=True, text=True)
        print(r.stdout.strip())
        assert r.returncode == 0, r.stderr
        return r.stdout

    eb = g.EDITBOX("Bonjour à tous", "WHISPER", "Hans Ysondre")
    g.SIMULATE_SEND(eb)
    assert len(g.BLIZZ_SENT) == 2 and eb.text == "" and not eb.shown, "Maj+Entrée a laissé partir le texte"
    out = app(frames_until_idle("s3"))
    # La boîte est rouverte, vide, sur le même chuchotement, et l'application en est prévenue.
    assert eb.shown and eb.focus and eb.text == "", "boîte de saisie non rouverte"
    assert eb.attrs.chatType == "WHISPER" and eb.attrs.tellTarget == "Hans Ysondre", "canal ou destinataire perdu"
    assert "[S3] envoi 1 (WHISPER à Hans Ysondre) : Bonjour à tous" in out, "demande non reçue"
    assert "[S3] collage prêt pour l'envoi 1 : [→DE] Bonjour à tous" in out, "collage non prévu"
    # L'application colle la traduction puis appuie sur Entrée (le joueur tient encore Maj) : elle part.
    eb.text = "[→DE] Bonjour à tous"
    g.ChatFrame1EditBox.hooks["OnTextChanged"](eb, True)  # collage : tout d'un coup, pas une frappe
    g.SIMULATE_SEND(eb)
    sent = list(g.BLIZZ_SENT.values())
    print("Envoyé S3 :", sent[-1])
    assert sent[-1] == "WHISPER:Hans Ysondre:[→DE] Bonjour à tous", "envoi traduit incorrect"
    out = app(frames_until_idle("s3sent"))
    assert "[S3] envoi 1 parti" in out, "état « parti » non transmis"
    g.SHIFT = False
    # Le joueur reprend la frappe pendant l'attente : annulé.
    eb = g.EDITBOX("Une autre phrase", "SAY", None)
    g.SHIFT = True
    g.SIMULATE_SEND(eb)
    g.SHIFT = False
    s3_frames.extend(frames_until_idle("s3tick", 1))
    assert eb.shown, "boîte non rouverte"
    n_log = len(g.LOG)
    eb.text = "a"
    g.ChatFrame1EditBox.hooks["OnTextChanged"](eb, True)
    assert len(g.LOG) == n_log, "annulation : pas de message dans le chat"
    out = app(frames_until_idle("s3cancel"))
    assert "[S3] envoi 2 annulé dans le jeu" in out, "annulation non transmise"
    # Traduction jamais arrivée : le texte d'origine revient dans la boîte au bout de 6 s.
    eb = g.EDITBOX("Personne ne répond", "SAY", None)
    g.SHIFT = True
    g.SIMULATE_SEND(eb)
    g.SHIFT = False
    s3_frames.extend(frames_until_idle("s3tick", 1))
    g.TICK(7)
    assert eb.text == "Personne ne répond", "texte d'origine non remis"
    assert "Traduction pas arrivée à temps" in list(g.LOG.values())[-1]
    s3_frames.extend(frames_until_idle("s3timeout"))
    # Lien d'objet : collé sous forme de repère {1}, remis par l'addon avant l'envoi.
    link = "|cffa335ee|Hitem:19019::::::::60:::::|h[Tonnerre]|h|r"
    eb = g.EDITBOX(f"Je vends {link} pas cher", "WHISPER", "Hans Ysondre")
    g.SHIFT = True
    g.SIMULATE_SEND(eb)
    g.SHIFT = False
    out = app(frames_until_idle("s3link"))
    # (Ids : la lettre tapée pendant l'annulation est partie en brouillon, n°3.)
    assert "[S3] collage prêt pour l'envoi 5 : [→DE] Je vends {1} pas cher" in out, "lien non remplacé par un repère"
    eb.text = "[→DE] Je vends {1} pas cher"
    g.ChatFrame1EditBox.hooks["OnTextChanged"](eb, True)  # collage : tout d'un coup, pas une frappe
    g.SIMULATE_SEND(eb)
    assert list(g.BLIZZ_SENT.values())[-1] == f"WHISPER:Hans Ysondre:[→DE] Je vends {link} pas cher", list(g.BLIZZ_SENT.values())[-1]
    s3_frames.extend(frames_until_idle("s3linksent"))
    # ---- Brouillon : traduit pendant la frappe ---------------------------------------------
    eb = g.EDITBOX("Merci pour le groupe", "WHISPER", "Hans Ysondre")
    g.ChatFrame1EditBox.hooks["OnTextChanged"](eb, True)
    out = app(frames_until_idle("draft"))
    assert "[S3] brouillon" in out and "Merci pour le groupe" in out, "brouillon non reçu"
    g.SHIFT = True
    g.SIMULATE_SEND(eb)
    g.SHIFT = False
    s3_frames.extend(frames_until_idle("s3tick", 1))
    eb.text = "[→DE] Merci pour le groupe"
    g.ChatFrame1EditBox.hooks["OnTextChanged"](eb, True)  # collage : tout d'un coup, pas une frappe
    g.SIMULATE_SEND(eb)
    assert list(g.BLIZZ_SENT.values())[-1] == "WHISPER:Hans Ysondre:[→DE] Merci pour le groupe"
    assert eb.text == "", "le texte d'origine est parti"
    out = app(frames_until_idle("draftsent"))
    assert "[S3] envoi 7 parti" in out, out
    # Langue choisie à la main puis envoi : retour en automatique 60 s plus tard.
    assert g.ChatFrame1EditBox.ctTrad.text == "Trad:DE"
    g.TICK(30)
    assert g.ChatFrame1EditBox.ctTrad.text == "Trad:DE", "retour en automatique trop tôt"
    g.TICK(31)
    assert g.ChatFrame1EditBox.ctTrad.text == "Trad:AUTO", "pas de retour en automatique après 60 s"
    g.SLASH("lang de")

    # ---- S5 : chat miroir -------------------------------------------------------
    def collect(tag, max_ticks=900, idle=40):
        files, quiet = [], 0
        for _ in range(max_ticks):
            before = banner.paints or 0
            g.TICK(1 / 60)
            if (banner.paints or 0) != before and banner.shown:
                raw = os.path.join(work, f"{tag}_{len(files):03d}.bgra")
                with open(raw, "wb") as f:
                    f.write(render_banner(banner, w, h))
                files.append(raw)
                quiet = 0
            else:
                quiet += 1
                if quiet >= idle and files:
                    break
        return files

    def feed(files, out):
        police = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"
        r = subprocess.run([binary, "feed", "--w", str(w), "--h", str(h), "--miroir-out", out,
                            "--police", police] + files, capture_output=True, text=True)
        print(r.stdout.strip())
        assert r.returncode == 0, r.stderr
        return r.stdout

    collect("avant")  # vide la file
    g.SLASH("miroir on")
    g.SLASH("miroir echantillon")
    assert g.OFFICIAL_ALPHA() == 0, "texte officiel non caché"
    files = collect("miroir")
    out = feed(files, os.path.join(work, "miroir"))
    assert "[miroir] ChatFrame1 : 387x108 px en (29, 86), Fonts\\ARIALN.TTF 12.6 px, ligne 14.0 px" in out, "description"
    assert "signe de vie de l'addon" in out, "pas de signe de vie avec le miroir actif"
    assert "[miroir 1] [Νίκος] chuchote : Καλησπέρα!" in out, "ligne grecque"
    assert "Vând [Thunderfury] ieftin" in out, "lien"
    assert "image 387x108" in out, "image du miroir"
    assert "[miroir] mesure du jeu : Fonts\\ARIALN.TTF 12.6 px : " in out, "avances mesurées"
    assert "retrait des lignes coupées : 10.8 px" in out, "retrait mesuré"
    assert "retrait 10.8 px (mesuré)" in out, "mise en page"
    assert "rapport police/jeu" in out
    assert "ChatFrame2" not in out, "le journal de combat a été touché"
    # 0.14.4 : ligne plus haute chez WoW (icône de qualité dans le texte) : l'écart de hauteur part
    # avec la description, l'application garde les lignes des joueurs à leur place.
    lua.execute("ChatFrame1.visibleLines[1].GetHeight = function(self) return 15.5 * self:GetNumLines() + 6 end")
    out = feed(collect("hauteur"), os.path.join(work, "hauteur"))
    assert "1 ligne(s) plus haute(s) chez WoW (icônes)" in out, out
    lua.execute("ChatFrame1.visibleLines[1].GetHeight = nil")
    # Bulle d'aide posée sur le chat : son rectangle part à l'application.
    g.GameTooltip.visible = True
    out = feed(collect("bulle"), os.path.join(work, "bulle"))
    assert "1 menu(s), bulle(s) ou fenêtre(s) du jeu sur le chat" in out, "bulle d'aide non signalée"
    g.GameTooltip.visible = False
    collect("bulle-fin")
    # ---- 0.12.3 : chat très actif, onglets, icônes ----------------------------------------
    # La fenêtre visible affiche /dire et les canaux Général et Commerce, pas Services.
    lua.execute("""
      ChatFrame1.IsEventRegistered = function(self, e) return e == "CHAT_MSG_SAY" or e == "CHAT_MSG_CHANNEL" end
      ChatFrame1.channelList = { "Général", "Commerce" }
      C_Texture = { GetAtlasInfo = function(name)
        if name:find("Quality") then return { file = 4559321, leftTexCoord = 0, rightTexCoord = 0.5,
          topTexCoord = 0, bottomTexCoord = 1, width = 34, height = 30 } end
      end }
      GetFileIDFromPath = function(p) if p:find("RaidTargetingIcon_8") then return 137008 end end
    """)
    g.SLASH("chat on")
    spam = "WTS boost Mythique+ ce soir, prix imbattables, chuchotez-moi vite"
    g.FIRE("CHAT_MSG_CHANNEL", spam, "Vendeur-Hyjal", "", "4. Services", "", "", 0, 4, "Services", 0, 1, "Player-2")
    item = "|cnIQ4:|Hitem:191304::::::::70|h[Fiole |A:Professions-ChatIcon-Quality-Tier3:17:15::1|a]|h|r"
    trade = f"Quelqu'un vend des {item} ou des fioles de rang 2 ?"
    g.FIRE("CHAT_MSG_CHANNEL", trade, "Acheteur-Hyjal", "", "2. Commerce", "", "", 0, 2, "Commerce", 0, 2, "Player-3")
    g.ADD(f"[2. Commerce] [Acheteur]: {trade}", "CHANNEL2", 1, 0.75, 0.75)
    raid = "On tape la cible |TInterface\\TargetingFrame\\UI-RaidTargetingIcon_8:0|t en premier, merci"
    g.FIRE("CHAT_MSG_SAY", raid, "Chef-Hyjal", "Commun", "", "", "", 0, 0, "", 7, 3, "Player-4")
    g.ADD(f"[Chef] dit : {raid}", "SAY")
    # Butin et système : laissés à WoW (ni texte ni mesure envoyés, juste leur place).
    g.ADD("Zalom crée : [Flacon des Magistères fugace].", "LOOT", 1, 1, 0)
    g.ADD("Vous avez appris un nouveau sort : [Imposition des mains].", "SYSTEM", 1, 1, 0)
    g.ADD("Résident de Lune-d'Argent dit : Vous semblez être quelqu'un de bien.", "MONSTER_SAY", 1, 1, 0.6)
    lua.execute("for _, fs in ipairs(ChatFrame1.visibleLines) do if fs.messageInfo then _G.ALPHAS = (_G.ALPHAS or '') .. fs:GetAlpha() end end")
    print("Opacité des lignes (bas en haut) :", g.ALPHAS)
    assert g.ALPHAS.startswith("11100"), "créatures, butin et système doivent rester dessinés par WoW, les joueurs cachés"
    busy = collect("busy")
    out = feed(busy, os.path.join(work, "busy"))
    cited = ct.Mirror.stats.cited or 0
    print("Lignes citant leur message :", cited)
    assert cited >= 2, "les lignes neuves n'ont pas cité leur message relayé"
    print("Icônes envoyées :", ct.Mirror.stats.icons)
    assert "Quelqu'un vend des [Fiole ] ou des fioles de rang 2 ?" in out, "citation non rétablie par l'application"
    assert "On tape la cible  en premier" in out, "ligne avec marque de raid"
    # Le message de Services (onglet non affiché) passe après ceux de la fenêtre visible.
    i_trade, i_spam = out.find("Acheteur-Hyjal: Quelqu'un"), out.find("Vendeur-Hyjal: WTS boost")
    assert 0 <= i_trade < i_spam, "le message d'un onglet caché est passé devant"
    assert "[icône] atlas Professions-ChatIcon-Quality-Tier3 : fichier 4559321, 34x30" in out, "atlas non décrit"
    assert "[icône] texture Interface\\TargetingFrame\\UI-RaidTargetingIcon_8 : fichier 137008" in out, "texture non décrite"
    assert "trame(s) manquée(s)" not in out
    assert "Zalom crée" not in out and "Imposition des mains" not in out and "Résident" not in out, "ligne laissée à WoW envoyée"
    if os.environ.get("CT_MIROIR_COPIE"):
        shutil.copy(os.path.join(work, "busy", sorted(os.listdir(os.path.join(work, "busy")))[-1]),
                    os.environ["CT_MIROIR_COPIE"].replace(".bmp", "_icones.bmp"))
    lua.execute("ChatFrame1.IsEventRegistered = nil; ChatFrame1.channelList = nil; C_Texture = nil; GetFileIDFromPath = nil")
    MIRROR_IMAGE = os.path.join(work, "miroir", "miroir_1.bmp")
    print("Image du miroir :", MIRROR_IMAGE)
    if os.environ.get("CT_MIROIR_COPIE"):
        shutil.copy(MIRROR_IMAGE, os.environ["CT_MIROIR_COPIE"])

    # Ligne secrète hors verrouillage (reçue pendant un combat, 0.13.3) : laissée à WoW, comme
    # une ligne du jeu ; les autres lignes de joueurs restent dessinées par l'application.
    g.ChatFrame1.AddMessage(g.ChatFrame1, g.SECRET)
    out = feed(collect("secret"), os.path.join(work, "secret"))
    assert g.SECRET_ALPHA() == 1, "ligne secrète cachée"
    assert g.OFFICIAL_ALPHA() == 0, "toute la fenêtre rendue au texte officiel pour une ligne secrète"
    assert "texte officiel (verrouillage ou ligne secrète)" not in out
    for i in range(10):
        g.ADD(f"ligne {i}", "SAY")
    collect("apres")
    assert g.OFFICIAL_ALPHA() == 0, "texte officiel toujours visible une fois la ligne secrète sortie"
    # Verrouillage du chat : texte officiel.
    g.LOCKDOWN = True
    collect("verrou")
    assert g.OFFICIAL_ALPHA() == 1, "texte officiel non rétabli (verrouillage)"
    g.LOCKDOWN = False
    collect("fin-verrou")
    assert g.OFFICIAL_ALPHA() == 0
    # 0.13.3 : message reçu pendant le verrouillage (texte et expéditeur secrets) : gardé sans
    # être lu, relayé à la fin du verrouillage s'il est devenu lisible (trame « late »), avec une
    # note pour l'application.
    g.LOCKDOWN = True
    lua.execute('SECRETS["Need a tank for the last boss please"] = true; SECRETS["Gorrak-Hyjal"] = true')
    held0, late0 = g.CT_HELD().held, g.CT_HELD().late
    g.FIRE("CHAT_MSG_SAY", "Need a tank for the last boss please", "Gorrak-Hyjal", "Commun", "", "", "", 0, 0, "", 0, 42, "Player-1-0")
    assert g.CT_HELD().held == held0 + 1, "message secret non gardé"
    collect("combat")
    g.LOCKDOWN = False
    lua.execute("SECRETS = {}")
    late = collect("apres-combat", max_ticks=300, idle=60)
    out = feed(late, os.path.join(work, "apres-combat"))
    assert g.CT_HELD().late == late0 + 1, "message gardé non relayé après le combat"
    assert "combat : 1 messages reçus pendant le verrouillage du chat, 1 relayés après, 0 restés secrets" in out, out
    assert "Gorrak-Hyjal: Need a tank for the last boss please" in out, "message du combat non relayé"
    # Bandeau immobile quand il n'y a rien à envoyer : la trame de vie reste à l'écran, sans
    # nouvelle peinture (0.13.3 : plus de clignotement).
    collect("calme", max_ticks=200, idle=30)
    before = banner.paints or 0
    for _ in range(180):
        g.TICK(1 / 60)
    assert banner.shown, "bandeau caché au repos (le miroir ne verrait plus le jeu)"
    # Au plus le rappel de la description du chat (toutes les 15 s) : avant, une trame de vie
    # toutes les 0,4 s faisait clignoter le bandeau (7 peintures en 3 s).
    assert (banner.paints or 0) - before <= 3, f"bandeau repeint au repos : {(banner.paints or 0) - before} fois en 3 s"
    # Contexte changé (entrée en combat) : une nouvelle trame de vie.
    before = banner.paints or 0
    g.IN_COMBAT = True
    for _ in range(60):
        g.TICK(1 / 60)
    assert (banner.paints or 0) > before, "contexte (combat) non envoyé"
    g.IN_COMBAT = False
    # 0.14.2 : réglage graphique modifié dans le jeu -> trame de vie repeinte avec son numéro ;
    # l'application réévalue la place du modèle dans la mémoire vidéo.
    collect("avant-gfx", max_ticks=200, idle=30)
    before = banner.paints or 0
    g.FIRE("CVAR_UPDATE", "chatBubbles", "1")
    for _ in range(60):
        g.TICK(1 / 60)
    assert (banner.paints or 0) == before, "réglage sans rapport avec le graphisme : trame de vie repeinte"
    g.FIRE("CVAR_UPDATE", "graphicsQuality", "7")
    out = feed(collect("gfx", max_ticks=200, idle=30), os.path.join(work, "gfx"))
    assert "[contexte] réglages graphiques de WoW modifiés" in out, out
    # 0.14.3 : en champ de bataille, UnitIsAFK et IsInInstance rendent des valeurs secrètes : ni
    # testées ni comptées (seule l'instance lisible compte).
    lua.execute('UnitIsAFK = function() return SECRET end; IsInInstance = function() return true, SECRET end')
    assert lua.eval("CT_TABLE.ContextFlags()") == 2, lua.eval("CT_TABLE.ContextFlags()")
    lua.execute('IsInInstance = function() return SECRET, "pvp" end')
    assert lua.eval("CT_TABLE.ContextFlags()") == 0
    lua.execute('UnitIsAFK = nil; IsInInstance = function() return false, "none" end')
    g.SLASH("miroir off")
    off = collect("off")
    assert off, "aucune description envoyée à l'arrêt du miroir"
    assert g.OFFICIAL_ALPHA() == 1, "texte officiel non rétabli (off)"
    feed(off, os.path.join(work, "off"))

    # ---- S4 : images par seconde pour le test des modèles ------------------------
    g.SLASH("perf on")
    g.TICK(0.3)  # une image de 300 ms : à-coup à signaler
    perf = collect("perf", max_ticks=120, idle=200)
    g.SLASH("perf off")
    g.SLASH("essai 4b")
    perf += collect("essai", max_ticks=120, idle=40)
    g.SLASH("essai")
    perf += collect("serie", max_ticks=120, idle=40)
    out = feed(perf, os.path.join(work, "perf"))
    assert "[demande] essai 4b" in out, "demande d'essai non transmise"
    assert "[demande] essai tout" in out, "demande de série non transmise"
    assert "[perf] 44.0 images/s, plus longue image 300 ms" in out, "images par seconde ou à-coups non transmis"

    g.SLASH("info")
    print("Info :", list(g.LOG.values())[-7:])
    info = list(g.LOG.values())
    assert any("Police du chat : Fonts\\ARIALN.TTF 14 ombre 1,-1 - client frFR" in m for m in info), "ligne police absente"
    g.SLASH("font")
    font = list(g.LOG.values())[len(info):]
    print("Police :", font)
    assert any("ChatFrame1 « Général » : Fonts\\ARIALN.TTF 14 ombre 1,-1 - 387x108 px, coin bas-gauche (29, 86)" in m for m in font), "géométrie ChatFrame1"
    assert any("ChatFrame2 « Journal de combat » (journal de combat, lecture seule)" in m for m in font), "journal de combat"
    assert not any("ChatFrame3" in m for m in font), "fenêtre masquée affichée"
    assert not any("Erreur" in m for m in font), "erreur Lua dans /chevaliers font"
    g.SLASH("trad on")
    assert any("Traduction du chat active" in m for m in list(g.LOG.values())[-3:]), "/chevaliers trad on"
    g.SLASH("trad off")
    assert any("Traduction du chat arrêtée" in m for m in list(g.LOG.values())[-3:]), "/chevaliers trad off"
    assert g.ChevaliersTranslatorsDB.tradOff is True, "trad off non gardé"
    # ---- Application lancée après la connexion : signe d'attente -------------------------------
    ct.waiting = True
    g.TICK(10)
    out = feed(collect("attente", max_ticks=120, idle=60), os.path.join(work, "attente"))
    assert "[présence] l'addon attend l'application (/reload)" in out, "signe d'attente non reçu"
    ct.waiting = None
    # ---- Diagnostic (retail) ---------------------------------------------------------------
    g.SLASH("diag")
    out = feed(collect("diag"), os.path.join(work, "diag"))
    assert "[addon] diag : client 1.60.1.70790" in out and "[addon] diag : ChatFrame1" in out, "diagnostic"
    assert "[addon] diag : messages reçus (total/secrets) :" in out and "WHISPER 1/1" in out, out
    # ---- Connexion : traduction active d'office si l'application tourne ---------------------
    first = list(g.LOG.values())
    assert any("l'application Chevaliers Translators n'est pas lancée" in m for m in first[:12]), "connexion sans l'application"
    g.FIRE("PLAYER_LOGIN")
    assert "traduction du chat coupée" in list(g.LOG.values())[-1], "trad off gardé à la connexion"
    g.ChevaliersTranslatorsDB.tradOff = None
    lua.execute('CHEVALIERS_LIVE = { t = time() - 3, v = "0.11.0" }')
    g.FIRE("PLAYER_LOGIN")
    assert "Traduction du chat active (application 0.11.0)" in list(g.LOG.values())[-1], list(g.LOG.values())[-1]
    g.ADD("[Hans] dit : Hallo", "SAY")
    assert g.OFFICIAL_ALPHA() == 0, "miroir non activé à la connexion"
    lua.execute('CHEVALIERS_LIVE = { t = time() - 60, v = "0.11.0" }')
    g.FIRE("PLAYER_LOGIN")
    assert "l'application Chevaliers Translators n'est pas lancée" in list(g.LOG.values())[-1], "application arrêtée depuis 1 min"
    # 0.13.1 : présence jugée au moment où le jeu lit le fichier (lu), pas à la fin de l'écran de
    # chargement. Fichier de présence réel, écrit par l'installation (t = 0 : application arrêtée).
    live_path = os.path.join(addons, "ChevaliersTranslators_Live", "Live.lua")
    live_src = open(live_path, encoding="utf-8").read()
    assert "t =            0," in live_src and "lu = time and time()" in live_src, live_src
    run_live = lua.eval("function(s) assert(loadstring(s, 'Live.lua'))() end")
    run_live(live_src)
    g.FIRE("PLAYER_LOGIN")
    assert "n'est pas lancée" in list(g.LOG.values())[-1], "présence à zéro (application quittée)"
    # Application lancée : fichier lu à la connexion, PLAYER_LOGIN 75 s plus tard (long chargement).
    import time as _time
    run_live(live_src.replace("t =            0,", "t = %12d," % (int(_time.time()) - 1)))
    assert g.CHEVALIERS_LIVE.lu is not None and abs(g.CHEVALIERS_LIVE.lu - int(_time.time())) <= 2, "lu"
    lua.execute("local real = time; time = function() return real() + 75 end; _G.REAL_TIME = real")
    before = len(list(g.LOG.values()))
    g.FIRE("PLAYER_LOGIN")
    lua.execute("time = REAL_TIME")
    said = list(g.LOG.values())[before:]
    assert said and "Traduction du chat active" in said[-1], f"long chargement : {said}"
    assert not any("/reload" in m for m in said), said
    # 0.14, dispositif télévision : l'application a vu WoW sur une télévision (e = 1) ; le texte
    # du chat du jeu n'est pas caché, la description demande de le recouvrir.
    assert "e = 0," in live_src, live_src
    run_live(live_src.replace("t =            0,", "t = %12d," % (int(_time.time()) - 1)).replace("e = 0,", "e = 1,"))
    g.FIRE("PLAYER_LOGIN")
    assert "Écran de télévision" in list(g.LOG.values())[-1], list(g.LOG.values())[-1]
    g.ADD("[Hans] dit : Hallo zusammen", "SAY")
    out = feed(collect("tele"), os.path.join(work, "tele"))
    assert g.OFFICIAL_ALPHA() == 1, "texte du jeu caché sur une télévision"
    assert "dispositif télévision (texte du jeu recouvert)" in out, out
    g.SLASH("tele off")
    assert "inactif" in list(g.LOG.values())[-1], list(g.LOG.values())[-1]
    collect("tele-off")
    assert g.OFFICIAL_ALPHA() == 0, "/chevaliers tele off : texte du jeu non caché"
    g.SLASH("tele auto")
    collect("tele-auto")
    assert g.OFFICIAL_ALPHA() == 1, "/chevaliers tele auto : télévision oubliée"
    run_live(live_src.replace("t =            0,", "t = %12d," % (int(_time.time()) - 1)))
    g.FIRE("PLAYER_LOGIN")
    collect("ecran-pc")
    assert g.OFFICIAL_ALPHA() == 0, "écran de PC : texte du jeu non caché"
    # ---- Commandes du joueur (J1) : aide, on/off, langue ; outils de développement à part ------
    g.SLASH("")
    helpmsg = list(g.LOG.values())[-3:]
    assert any("/chevaliers on | off" in m for m in helpmsg), helpmsg
    assert not any("essai" in m or "miroir" in m for m in helpmsg), "outils de développement dans l'aide"
    g.SLASH("dev")
    assert "miroir on|calage|off" in list(g.LOG.values())[-1], "/chevaliers dev"
    g.SLASH("off")
    assert any("Traduction du chat arrêtée" in m for m in list(g.LOG.values())[-3:]), "/chevaliers off"
    assert g.ChevaliersTranslatorsDB.tradOff is True, "/chevaliers off non gardé"
    g.SLASH("on")
    assert any("Traduction du chat active" in m for m in list(g.LOG.values())[-3:]), "/chevaliers on"
    assert g.ChevaliersTranslatorsDB.tradOff is None, "/chevaliers on non gardé"
    g.SLASH("langue it")
    assert g.ChevaliersTranslatorsDB.dest == "IT", f"/chevaliers langue it : {g.ChevaliersTranslatorsDB.dest}"
    g.SLASH("n'importe quoi")
    assert any("/chevaliers on | off" in m for m in list(g.LOG.values())[-3:]), "commande inconnue : aide"
    # Réponses à une proposition de modèle (0.14) : transmises à l'application.
    g.SLASH("monter")
    assert "proposé sera essayé hors combat" in list(g.LOG.values())[-1]
    g.SLASH("non")
    assert "ne te sera plus proposé" in list(g.LOG.values())[-1]
    print(f"Harnais OK : {decoded} trames peintes par l'addon décodées par ct-outils")


if __name__ == "__main__":
    mode, root = sys.argv[1], sys.argv[2]
    if mode == "vectors":
        vectors(root)
    elif mode == "mock":
        lint_secret_order(root)
        mock(root, sys.argv[3])
    else:
        sys.exit(__doc__)
