-- SimonUI v8.1 — consolidated HUD: mode machine, self-healing round
-- timer, word-input bar (submit on ANY focus loss), zone hint pill.
-- Era-safe classes only (no UICorner/UIStroke/TweenService).
print("[SimonSays] SimonUI v8.1 loaded")
local Players  = game:GetService("Players")
local player   = Players.LocalPlayer
local simon    = workspace:WaitForChild("SimonSays")
local remotes  = simon:WaitForChild("Remotes")
local messages = simon:WaitForChild("Messages")
local config   = simon:WaitForChild("Configurations")

local gui        = player:WaitForChild("PlayerGui"):WaitForChild("SimonSaysUI")
local root       = gui:WaitForChild("PanelRoot")
local border     = root.Panel:WaitForChild("Border")
local status     = root.Panel:WaitForChild("Status")
local countdown  = root.Panel:WaitForChild("CountdownLabel")
local timerFill  = root.Panel.TimerBar:WaitForChild("Fill")
local banner     = gui:WaitForChild("SimonBanner")
local pill       = banner:WaitForChild("Pill")
local pillShadow = banner:WaitForChild("PillShadow")
local pillBorder = pill:WaitForChild("Border")
local simonLabel = pill:WaitForChild("SimonLabel")
local tickSound  = gui:WaitForChild("TickSound")
local dingSound  = gui:WaitForChild("DingSound")
local minPlayersValue = config:WaitForChild("MinimalPlayers")
local wordInputUI = remotes:WaitForChild("WordInputUI")
local wordSubmit  = remotes:WaitForChild("WordSubmit")
local zoneHint    = remotes:WaitForChild("ZoneHint")

local ACCENT = border.BorderColor3
local WHITE  = Color3.new(1, 1, 1)
local RED    = Color3.fromRGB(255, 60, 60)
local WINDOW = Color3.fromRGB(30, 26, 22)
local FIELD  = Color3.fromRGB(45, 38, 30)
local TEAM_ALIVE  = BrickColor.new(21) -- must match Settings.TEAM_ALIVE
local TEAM_WINNER = BrickColor.new(37)

local PANEL_SHOWN_Y  = 10
local PANEL_HIDDEN_Y = -130
local PILL_W, PILL_H = 640, 64

------------------------------------------------------------------ self-healing banner timer frames
local function ensureBannerTimer()
    local track = pill:FindFirstChild("BannerTimerTrack")
    if not track then
        track = Instance.new("Frame")
        track.Name = "BannerTimerTrack"
        track.AnchorPoint = Vector2.new(0.5, 1)
        track.Position = UDim2.new(0.5, 0, 1, -3)
        track.Size = UDim2.new(0, 180, 0, 3)
        track.BackgroundColor3 = Color3.fromRGB(60, 52, 42)
        track.BorderSizePixel = 0
        track.ZIndex = 11
        track.Visible = false
        track.Parent = pill

        local fill = Instance.new("Frame")
        fill.Name = "Fill"
        fill.Size = UDim2.new(0, 0, 1, 0)
        fill.BackgroundColor3 = ACCENT
        fill.BorderSizePixel = 0
        fill.ZIndex = 12
        fill.Parent = track
    end

    local label = pill:FindFirstChild("BannerTimerLabel")
    if not label then
        label = Instance.new("TextLabel")
        label.Name = "BannerTimerLabel"
        label.AnchorPoint = Vector2.new(0.5, 1)
        label.Position = UDim2.new(0.5, 0, 1, -7)
        label.Size = UDim2.new(0, 220, 0, 13)
        label.BackgroundTransparency = 1
        label.TextColor3 = ACCENT
        label.Font = Enum.Font.SourceSans
        label.TextSize = 12
        label.Text = ""
        label.ZIndex = 11
        label.Visible = false
        label.Parent = pill
    end

    simonLabel.Size = UDim2.new(1, -32, 1, -22)
    return track:WaitForChild("Fill"), label
end

local bannerTimerTrack, bannerTimerLabel = ensureBannerTimer()
local bannerTimerFill = bannerTimerTrack

------------------------------------------------------------------ animation kit
local function animate(duration, step)
    local start = tick()
    while true do
        local a = (tick() - start) / duration
        if a >= 1 then break end
        step(a)
        wait(1 / 30)
    end
    step(1)
end

local function lerpNumber(a, b, t)
    return a + (b - a) * t
end

local function lerpColor(a, b, t)
    return Color3.new(
        a.R + (b.R - a.R) * t,
        a.G + (b.G - a.G) * t,
        a.B + (b.B - a.B) * t)
end

local slideToken, popToken, flashToken = 0, 0, 0

local function slidePanel(show)
    slideToken = slideToken + 1
    local myToken = slideToken
    local fromY = root.Position.Y.Offset
    local toY = show and PANEL_SHOWN_Y or PANEL_HIDDEN_Y
    if show then
        root.Visible = true
    end
    animate(0.25, function(a)
        if myToken ~= slideToken then return end
        root.Position = UDim2.new(0.5, 0, 0, lerpNumber(fromY, toY, a))
    end)
    if not show and myToken == slideToken then
        root.Visible = false
    end
end

local function barController(fillFrame)
    local maxT = 0
    local token = 0
    local function set(value)
        if maxT <= 0 then maxT = value end
        if maxT <= 0 then return end
        local ratio = math.min(value / maxT, 1)
        token = token + 1
        local myToken = token
        local from = fillFrame.Size.X.Scale
        animate(0.25, function(a)
            if myToken ~= token then return end
            fillFrame.Size = UDim2.new(lerpNumber(from, ratio, a), 0, 1, 0)
        end)
    end
    local function reset()
        maxT = 0
        token = token + 1
        fillFrame.Size = UDim2.new(0, 0, 1, 0)
    end
    return set, reset
end

local setPanelTimer, resetPanelTimer = barController(timerFill)
local setBannerBar, resetBannerBar = barController(bannerTimerFill)

local function popPill()
    popToken = popToken + 1
    local myToken = popToken
    animate(0.12, function(a)
        if myToken ~= popToken then return end
        local s = 0.92 + 0.08 * a
        pill.Size = UDim2.new(0, PILL_W * s, 0, PILL_H * s)
    end)
end

local function flashBorder()
    flashToken = flashToken + 1
    local myToken = flashToken
    animate(0.4, function(a)
        if myToken ~= flashToken then return end
        pillBorder.BorderColor3 = lerpColor(WHITE, ACCENT, a)
    end)
end

------------------------------------------------------------------ state
local timerActive = false
local mode = "hide" -- "notenough" | "intermission" | "ingame" | "hide"
local serverEligible = nil
local serverMin = nil

local function clientEligible()
    local n = 0
    for _, plr in ipairs(Players:GetPlayers()) do
        if plr.TeamColor ~= TEAM_WINNER then
            n = n + 1
        end
    end
    return n
end

local function countAlive()
    local n = 0
    for _, plr in ipairs(Players:GetPlayers()) do
        if plr.TeamColor == TEAM_ALIVE then
            n = n + 1
        end
    end
    return n
end

local function refreshStatus()
    if mode == "notenough" then
        status.Text = "Not enough players (" ..
            (serverEligible or clientEligible()) .. "/" .. (serverMin or minPlayersValue.Value) .. ")"
    elseif mode == "intermission" then
        status.Text = "Intermission — Players: " .. #Players:GetPlayers()
    elseif mode == "ingame" then
        status.Text = "In game — Alive: " .. countAlive()
    else
        status.Text = "Status: Waiting for players"
    end
end

------------------------------------------------------------------ WORD INPUT BAR (form, not chat)
local inputRoot = Instance.new("Frame")
inputRoot.Name = "WordInput"
inputRoot.AnchorPoint = Vector2.new(0.5, 1)
inputRoot.Position = UDim2.new(0.5, 0, 1, -14)
inputRoot.Size = UDim2.new(0, 360, 0, 54)
inputRoot.BackgroundTransparency = 1
inputRoot.Visible = false
inputRoot.Parent = gui

local inputShadow = Instance.new("Frame")
inputShadow.Name = "Shadow"
inputShadow.Position = UDim2.new(0, 3, 0, 3)
inputShadow.Size = UDim2.new(1, 0, 1, 0)
inputShadow.BackgroundColor3 = Color3.new(0, 0, 0)
inputShadow.BackgroundTransparency = 0.55
inputShadow.BorderSizePixel = 0
inputShadow.ZIndex = 1
inputShadow.Parent = inputRoot

local inputFrame = Instance.new("Frame")
inputFrame.Name = "Bar"
inputFrame.Size = UDim2.new(1, 0, 1, 0)
inputFrame.BackgroundColor3 = WINDOW
inputFrame.BorderSizePixel = 1
inputFrame.BorderColor3 = ACCENT
inputFrame.ZIndex = 2
inputFrame.Parent = inputRoot

local inputBox = Instance.new("TextBox")
inputBox.Name = "Field"
inputBox.Position = UDim2.new(0, 10, 0, 10)
inputBox.Size = UDim2.new(1, -110, 1, -20)
inputBox.BackgroundColor3 = FIELD
inputBox.BorderSizePixel = 0
inputBox.TextColor3 = WHITE
inputBox.Font = Enum.Font.SourceSans
inputBox.TextSize = 18
inputBox.TextXAlignment = Enum.TextXAlignment.Left
inputBox.ClearTextOnFocus = false
inputBox.Text = ""
inputBox.PlaceholderText = "Type the word here..."
inputBox.ZIndex = 3
inputBox.Parent = inputFrame

local enterButton = Instance.new("TextButton")
enterButton.Name = "EnterButton"
enterButton.Position = UDim2.new(1, -90, 0, 10)
enterButton.Size = UDim2.new(0, 80, 1, -20)
enterButton.BackgroundColor3 = ACCENT
enterButton.BorderSizePixel = 0
enterButton.TextColor3 = Color3.fromRGB(30, 26, 22)
enterButton.Font = Enum.Font.SourceSansBold
enterButton.TextSize = 18
enterButton.Text = "Enter"
enterButton.ZIndex = 3
enterButton.Parent = inputFrame

inputBox.Changed:Connect(function(prop)
    if prop == "Text" then
        tickSound:Play()
    end
end)

local submitting = false
local function submitWord()
    if submitting then return end
    local text = string.gsub(inputBox.Text, "^%s*(.-)%s*$", "%1")
    if text == "" then return end
    submitting = true
    wordSubmit:FireServer(text)
    print("[SimonSays] word submitted: " .. text)
    inputBox.Text = ""
    dingSound:Play()
    enterButton.Text = "Sent!"
    wait(0.6)
    enterButton.Text = "Enter"
    submitting = false
end

enterButton.MouseButton1Click:Connect(submitWord)
-- typing = saying: ANY focus loss submits (click away, tap button,
-- keyboard Enter). Only exact matches register server-side.
inputBox.FocusLost:Connect(function()
    submitWord()
end)

wordInputUI.OnClientEvent:Connect(function(show)
    if show then
        inputBox.Text = ""
        inputRoot.Visible = true
    else
        inputRoot.Visible = false
    end
end)

------------------------------------------------------------------ ZONE HINT (armed players, real rounds)
local hintLabel = Instance.new("TextLabel")
hintLabel.Name = "ZoneHint"
hintLabel.AnchorPoint = Vector2.new(0.5, 0)
hintLabel.Position = UDim2.new(0.5, 0, 0.5, 44)
hintLabel.Size = UDim2.new(0, 460, 0, 30)
hintLabel.BackgroundColor3 = WINDOW
hintLabel.BorderSizePixel = 1
hintLabel.BorderColor3 = ACCENT
hintLabel.TextColor3 = ACCENT
hintLabel.Font = Enum.Font.SourceSansBold
hintLabel.TextSize = 15
hintLabel.Text = "Step off and back on - being there already doesn't count!"
hintLabel.ZIndex = 11
hintLabel.Visible = false
hintLabel.Parent = banner

zoneHint.OnClientEvent:Connect(function(show)
    hintLabel.Visible = show and true or false
end)

------------------------------------------------------------------ remotes
remotes.IntermissionUI.OnClientEvent:Connect(function(newMode, eligible, min)
    if type(eligible) == "number" then serverEligible = eligible end
    if type(min) == "number" then serverMin = min end

    local wasMode = mode
    hintLabel.Visible = false -- safety: no stale hints across mode changes

    if newMode == "hide" or newMode == false then
        mode = "hide"
        countdown.Text = ""
        bannerTimerLabel.Visible = false
        bannerTimerTrack.Visible = false
        inputRoot.Visible = false
        slidePanel(false)
    elseif newMode == "notenough" then
        mode = "notenough"
        countdown.Text = ""
        inputRoot.Visible = false
        slidePanel(true)
    elseif newMode == "intermission" or newMode == true then
        mode = "intermission"
        if wasMode ~= "intermission" then
            resetPanelTimer()
            countdown.Text = ""
            countdown.TextColor3 = WHITE
        end
        inputRoot.Visible = false
        slidePanel(true)
    elseif newMode == "ingame" then
        mode = "ingame"
        if wasMode ~= "ingame" then
            resetPanelTimer()
            resetBannerBar()
            countdown.Text = ""
            countdown.TextColor3 = WHITE
            bannerTimerLabel.Visible = false
            bannerTimerTrack.Visible = false
        end
        slidePanel(true)
    end
    refreshStatus()
end)

remotes.UIMove.OnClientEvent:Connect(function(position, text)
    if position then
        banner.Position = position
    end
    simonLabel.Text = text or ""
    local hasText = text ~= nil and text ~= ""
    pill.Visible = hasText
    pillShadow.Visible = hasText
    if hasText then
        dingSound:Play()
        popPill()
        flashBorder()
    end
end)

messages.IntermissionValue.Changed:Connect(function(value)
    if mode ~= "intermission" then return end
    if value > 0 then
        countdown.Text = "Next round in: " .. value
        countdown.TextColor3 = WHITE
        setPanelTimer(value)
        if value <= 5 then
            tickSound:Play()
        end
    else
        resetPanelTimer()
    end
    refreshStatus()
end)

messages.ActionValue.Changed:Connect(function(value)
    if value > 0 then
        if mode ~= "ingame" then
            mode = "ingame"
            resetPanelTimer()
            resetBannerBar()
            countdown.TextColor3 = WHITE
            slidePanel(true)
        end
        if not timerActive then
            timerActive = true
            print("[SimonSays] client: round timer received (" .. value .. "s)")
        end

        countdown.Text = "" .. value
        countdown.TextColor3 = value <= 5 and RED or ACCENT
        setPanelTimer(value)

        bannerTimerLabel.Text = "ends in " .. value .. "s"
        bannerTimerLabel.TextColor3 = value <= 5 and RED or ACCENT
        bannerTimerLabel.Visible = true
        bannerTimerTrack.Visible = true
        setBannerBar(value)

        if value <= 5 then
            tickSound:Play()
        end
    else
        timerActive = false
        if mode == "ingame" then
            countdown.Text = ""
            resetPanelTimer()
            bannerTimerLabel.Visible = false
            bannerTimerTrack.Visible = false
            resetBannerBar()
        end
    end
    refreshStatus()
end)

Players.PlayerAdded:Connect(function() refreshStatus() end)
Players.PlayerRemoving:Connect(function() refreshStatus() end)

while true do
    wait(1)
    if root.Visible then
        refreshStatus()
    end
end
