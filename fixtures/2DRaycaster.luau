-- [patched: stripped cached math locals v1]
-- [[ CONFIGURATION ]] --
local GRID_SIZE = 35
local MAX_DISTANCE = 400
local UPDATE_THRESHOLD = 0.15
local ROTATION_THRESHOLD = 0.8
local MAX_DEPTH = 2
local IGNORE_LIST_UPDATE_INTERVAL = 6
local SCREEN_SCALE = 0.5
local MAX_TAG_RANGE = 20
local NAME_TAG_UPDATE_INTERVAL = 0.3
local LIGHT_INFLUENCE_SCALE = 0.2
local VERTICAL_BRIGHTNESS = 1.3
local VERTICAL_Y_THRESHOLD = 0.1
local COLOR_CHANGE_THRESHOLD_STILL = 0.005
local COLOR_CHANGE_THRESHOLD_MOVING = 0.06
local currentThreshold = COLOR_CHANGE_THRESHOLD_STILL

-- [[ SERVICES ]] --
local RunService = game:GetService("RunService")
local UserInputService = game:GetService("UserInputService")
local Players = game:GetService("Players")
local Lighting = game:GetService("Lighting")

local camera = workspace.CurrentCamera
local localPlayer = Players.LocalPlayer
local lightsFolder = workspace:FindFirstChild("Lights")
if not lightsFolder then
    lightsFolder = Instance.new("Folder")
    lightsFolder.Name = "Lights"
    lightsFolder.Parent = workspace
end
local screenGui = Instance.new("ScreenGui")
screenGui.Name = "Raycaster2D"
screenGui.Parent = script.Parent

local mainFrame = Instance.new("Frame")
mainFrame.Name = "MainFrame"
mainFrame.Size = UDim2.new(SCREEN_SCALE, 0, SCREEN_SCALE, 0)
mainFrame.Position = UDim2.new(0.5 - SCREEN_SCALE / 2, 0, 0.5 - SCREEN_SCALE / 2, 0)
mainFrame.BackgroundTransparency = 1
mainFrame.Parent = screenGui

local ConfigFrame = Instance.new("ScrollingFrame")
ConfigFrame.Name = "ConfigFrame"
ConfigFrame.Size = UDim2.new(0, 200, 0, 150)
ConfigFrame.Position = UDim2.new(0.5, -100, 0.5, -75)
ConfigFrame.BackgroundColor3 = Color3.new(0, 0, 0)
ConfigFrame.BackgroundTransparency = 0.3
ConfigFrame.Visible = false
ConfigFrame.Parent = screenGui

local promptText = Instance.new("TextLabel")
promptText.Name = "PromptText"
promptText.Size = UDim2.new(1, 0, 0, 20)
promptText.Position = UDim2.new(0, 0, 1, -20)
promptText.Text = "Press Q to configure"
promptText.TextColor3 = Color3.new(1, 1, 1)
promptText.BackgroundTransparency = 1
promptText.TextScaled = true
promptText.Parent = screenGui

local scrollPromptText = Instance.new("TextLabel")
scrollPromptText.Name = "ScrollPromptText"
scrollPromptText.Size = UDim2.new(1, 0, 0, 20)
scrollPromptText.Position = UDim2.new(0, 0, 0, 0)
scrollPromptText.Text = "Scroll to resize"
scrollPromptText.TextColor3 = Color3.new(1, 1, 1)
scrollPromptText.BackgroundTransparency = 1
scrollPromptText.TextScaled = true
scrollPromptText.Visible = false
scrollPromptText.Parent = screenGui

-- [[ CACHE & FAST MATH ]] --
local squares = {}
local squareColors = {}
local groups = {[0]={}, [1]={}, [2]={}, [3]={}}
local nameTags = {}
local chatBubbles = {}
local ignoreList = {}
local activeLightsData = {}

local lastIgnoreListUpdate = 0
local lastNameTagUpdate = 0
local lastCameraCFrame = camera.CFrame
local currentPhase = 0

local DAY_COLOR = Color3.new(0, 0.333333, 0.498039)
local NIGHT_COLOR = Color3.new(0.1, 0.1, 0.2)
local DEFAULT_COLOR = DAY_COLOR

local rayParams = RaycastParams.new()
rayParams.FilterType = Enum.RaycastFilterType.Exclude
rayParams.IgnoreWater = true

-- [[ UTILITIES ]] --
local function lerpColor(c1, c2, t)
    return Color3.new(c1.R + (c2.R-c1.R)*t, c1.G + (c2.G-c1.G)*t, c1.B + (c2.B-c1.B)*t)
end

local function updateDefaultColor()
    local t = Lighting.ClockTime
    if t >= 6 and t < 8 then
        DEFAULT_COLOR = lerpColor(NIGHT_COLOR, DAY_COLOR, (t - 6) / 2)
    elseif t >= 18 then
        DEFAULT_COLOR = NIGHT_COLOR
    else
        DEFAULT_COLOR = DAY_COLOR
    end
end

local function findFirstAncestorOfClass(instance, className)
    local parent = instance.Parent
    while parent do
        if parent:IsA(className) then return parent end
        parent = parent.Parent
    end
    return nil
end

-- [[ SETTINGS UI ]] --
local rowCount = 0
local function createSettingRow(name)
    rowCount = rowCount + 1
    local row = Instance.new("Frame")
    row.Size = UDim2.new(1, -8, 0, 20)
    row.Position = UDim2.new(0, 0, 0, (rowCount - 1) * 22)
    row.BackgroundTransparency = 1

    local label = Instance.new("TextLabel")
    label.Size = UDim2.new(0.5, 0, 1, 0)
    label.Text = name .. ":"
    label.TextScaled = true
    label.TextColor3 = Color3.new(1, 1, 1)
    label.BackgroundTransparency = 1
    label.Parent = row

    local textbox = Instance.new("TextBox")
    textbox.Size = UDim2.new(0.5, 0, 1, 0)
    textbox.Position = UDim2.new(0.5, 0, 0, 0)
    textbox.TextScaled = true
    textbox.BackgroundTransparency = 0.5
    textbox.BackgroundColor3 = Color3.new(0, 0, 0)
    textbox.TextColor3 = Color3.new(1, 1, 1)
    textbox.Parent = row

    row.Parent = ConfigFrame
    ConfigFrame.CanvasSize = UDim2.new(0, 0, 0, rowCount * 22)
    return textbox
end

local gridSizeTextBox = createSettingRow("Grid Size")
local screenScaleTextBox = createSettingRow("Screen Scale")

-- [[ CHARACTER & UI ]] --
local function makeHeadSpherical(character)
    local head = character:WaitForChild("Head", 5)
    if head and head:IsA("BasePart") then
        for _, child in ipairs(head:GetChildren()) do
            if child:IsA("SpecialMesh") then child:Destroy() end
        end
        head.Size = Vector3.new(2, 2, 2)
        head.Shape = Enum.PartType.Ball
        for _, acc in ipairs(character:GetChildren()) do
            if acc:IsA("Accessory") or acc:IsA("Hat") then acc:Destroy() end
        end
    end
end

local function createNameTag(entity, name)
    local tag = Instance.new("TextLabel")
    tag.Size = UDim2.new(0, 60, 0, 12)
    tag.BackgroundTransparency = 0.5
    tag.BackgroundColor3 = Color3.new(0, 0, 0)
    tag.TextColor3 = Color3.new(1, 1, 1)
    tag.TextScaled = true
    tag.Text = name
    tag.Visible = false
    tag.Parent = screenGui
    return tag
end

local function createChatBubble()
    local bubble = Instance.new("TextLabel")
    bubble.Size = UDim2.new(0, 200, 0, 40)
    bubble.BackgroundTransparency = 0.5
    bubble.BackgroundColor3 = Color3.new(0, 0, 0)
    bubble.TextColor3 = Color3.new(1, 1, 1)
    bubble.TextScaled = true
    bubble.TextWrapped = true
    bubble.Visible = false
    bubble.Parent = screenGui
    return bubble
end

-- [[ CORE LOGIC ]] --
local function manualWorldToViewportPoint(worldPos, camProps)
    local cameraSpace = camProps.CFrame:PointToObjectSpace(worldPos)
    if cameraSpace.Z > 0 then return Vector2.new(0, 0), false end
    local z = -cameraSpace.Z
    local x = cameraSpace.X / z
    local y = cameraSpace.Y / z
    local screenX = (x / camProps.AspectRatio + 1) * 0.5 * camProps.ViewportSize.X
    local screenY = (1 - y) * 0.5 * camProps.ViewportSize.Y
    return Vector2.new(screenX, screenY), (screenX >= 0 and screenX <= camProps.ViewportSize.X and screenY >= 0 and screenY <= camProps.ViewportSize.Y)
end

local function updateIgnoreList()
    local currentTime = tick()
    if currentTime - lastIgnoreListUpdate < IGNORE_LIST_UPDATE_INTERVAL then return end

    ignoreList = {}
    activeLightsData = {}

    if localPlayer.Character then
        for _, part in ipairs(localPlayer.Character:GetDescendants()) do
            if part:IsA("BasePart") and not findFirstAncestorOfClass(part, "Tool") then
                table.insert(ignoreList, part)
            end
        end
    end

    for _, model in ipairs(workspace:GetChildren()) do
        if model:IsA("Model") and model ~= localPlayer.Character then
            local hrp = model:FindFirstChild("HumanoidRootPart")
            if hrp then table.insert(ignoreList, hrp) end
        end
    end

    for _, part in ipairs(lightsFolder:GetChildren()) do
        if part:IsA("BasePart") then
            local light = part:FindFirstChildOfClass("PointLight")
            if light and light.Enabled then
                table.insert(activeLightsData, {
                    position = part.Position,
                    range = light.Range,
                    rangeSqr = light.Range * light.Range,
                    brightness = light.Brightness,
                    color = light.Color
                })
            end
        end
    end

    rayParams.FilterDescendantsInstances = ignoreList
    lastIgnoreListUpdate = currentTime
end

local function initializeGrid()
    for _, child in ipairs(mainFrame:GetChildren()) do
        if child:IsA("Frame") then child:Destroy() end
    end
    squares, squareColors = {}, {}
    groups = {[0]={}, [1]={}, [2]={}, [3]={}}

    local inv = 1 / GRID_SIZE
    for i = 1, GRID_SIZE do
        squares[i], squareColors[i] = {}, {}
        for j = 1, GRID_SIZE do
            local sq = Instance.new("Frame")
            sq.Size = UDim2.new(inv, 0, inv, 0)
            sq.Position = UDim2.new((i - 1) * inv, 0, (j - 1) * inv, 0)
            sq.BackgroundColor3 = DEFAULT_COLOR
            sq.BorderSizePixel = 0
            sq.Parent = mainFrame
            squares[i][j], squareColors[i][j] = sq, DEFAULT_COLOR

            local phase = (i % 2) + 2 * (j % 2)
            table.insert(groups[phase], {i, j})
        end
    end
end

-- [[ RENDERING ENGINE ]] --
local function updateGrid(phaseIndex, camProps, frameProps)
    local group_squares = groups[phaseIndex]
    local camCFrame, camPos, invG = camProps.CFrame, camProps.CFrame.p, 1/GRID_SIZE

    for _, ij in ipairs(group_squares) do
        local i, j = ij[1], ij[2]
        local x = frameProps.Pos.X + (i - 0.5) * invG * frameProps.Size.X
        local y = frameProps.Pos.Y + (j - 0.5) * invG * frameProps.Size.Y
        local nx = (2 * x / camProps.ViewportSize.X - 1) * camProps.AspectRatio
        local ny = 1 - 2 * y / camProps.ViewportSize.Y
        local dir = camCFrame:VectorToWorldSpace(Vector3.new(nx, ny, -1)).Unit

        local r, g, b = 0, 0, 0
        local transmission = 1
        local current_origin = camPos
        local total_distance = 0
        local depth = 0
        local lastOpaqueHitPos

        while transmission > 0.01 and depth < MAX_DEPTH and total_distance < MAX_DISTANCE do
            local raycastResult = workspace:Raycast(current_origin, dir * (MAX_DISTANCE - total_distance), rayParams)

            if not raycastResult then break end

            local hit = raycastResult.Instance
            local pos = raycastResult.Position
            local normal = raycastResult.Normal
            local T = hit.Transparency

            if T < 1 then
                local alpha = 1 - T
                local multiplier = alpha * transmission
                local pCol = hit.Color

                if math.abs(normal.Y) < VERTICAL_Y_THRESHOLD then
                    pCol = Color3.new(math.min(pCol.R * VERTICAL_BRIGHTNESS, 1), math.min(pCol.G * VERTICAL_BRIGHTNESS, 1), math.min(pCol.B * VERTICAL_BRIGHTNESS, 1))
                end

                r, g, b = r + pCol.R * multiplier, g + pCol.G * multiplier, b + pCol.B * multiplier
                transmission = transmission * T
                total_distance = total_distance + (pos - current_origin).Magnitude + 0.001
                lastOpaqueHitPos = pos
            end
            current_origin = pos + dir * 0.001
            depth = depth + 1
        end

        local acc_color = (r == 0 and g == 0 and b == 0) and DEFAULT_COLOR or Color3.new(r, g, b)

        if lastOpaqueHitPos then
            local amR, amG, amB = 0, 0, 0
            for _, l in ipairs(activeLightsData) do
                local off = lastOpaqueHitPos - l.position
                local d2 = off.X*off.X + off.Y*off.Y + off.Z*off.Z
                if d2 < l.rangeSqr then
                    local d = math.sqrt(d2)
                    local f = l.brightness * (1 - d/l.range) * LIGHT_INFLUENCE_SCALE
                    amR, amG, amB = amR + l.color.R*f, amG + l.color.G*f, amB + l.color.B*f
                end
            end
            acc_color = Color3.new(math.min(acc_color.R+amR, 1), math.min(acc_color.G+amG, 1), math.min(acc_color.B+amB, 1))
        end

        local cur = squareColors[i][j]
        if math.abs(acc_color.R - cur.R) > currentThreshold or math.abs(acc_color.G - cur.G) > currentThreshold or math.abs(acc_color.B - cur.B) > currentThreshold then
            squares[i][j].BackgroundColor3 = acc_color
            squareColors[i][j] = acc_color
        end
    end
end

-- [[ RUNTIME ]] --
local menuOpen = false

RunService.RenderStepped:Connect(function()
    updateIgnoreList()
    updateDefaultColor()
    local camCFrame = camera.CFrame
    local vSize = camera.ViewportSize
    local posDiff = (camCFrame.p - lastCameraCFrame.p).Magnitude
    local rotDiff = (camCFrame.LookVector - lastCameraCFrame.LookVector).Magnitude

    local phasesToUpdate = 1
    if posDiff > 1.5 or rotDiff > 0.4 then
        currentThreshold = COLOR_CHANGE_THRESHOLD_MOVING
        phasesToUpdate = 2
    elseif posDiff > UPDATE_THRESHOLD or rotDiff > ROTATION_THRESHOLD then
        currentThreshold = (COLOR_CHANGE_THRESHOLD_MOVING + COLOR_CHANGE_THRESHOLD_STILL)/2
        phasesToUpdate = 1
    else
        currentThreshold = COLOR_CHANGE_THRESHOLD_STILL
        phasesToUpdate = 1
    end

    local cP = {CFrame = camCFrame, ViewportSize = vSize, AspectRatio = vSize.X / vSize.Y}
    local fP = {Pos = mainFrame.AbsolutePosition, Size = mainFrame.AbsoluteSize}

    for _ = 1, phasesToUpdate do
        updateGrid(currentPhase, cP, fP)
        currentPhase = (currentPhase + 1) % 4
    end

    if tick() - lastNameTagUpdate >= NAME_TAG_UPDATE_INTERVAL then
        for entity, tag in pairs(nameTags) do
            local char = (entity:IsA("Player") and entity.Character) or entity
            if char and char:FindFirstChild("Head") then
                local hPos = char.Head.Position
                if (hPos - camCFrame.p).Magnitude > MAX_TAG_RANGE then
                    tag.Visible = false
                else
                    local sPos, vis = manualWorldToViewportPoint(hPos + Vector3.new(0, 2, 0), cP)
                    tag.Visible = vis
                    if vis then tag.Position = UDim2.new(0, sPos.X - 30, 0, sPos.Y - 6) end
                end
            end
        end
        lastNameTagUpdate = tick()
    end
    lastCameraCFrame = camCFrame
end)

-- [[ PLAYER INIT ]] --
local function setupPlayer(player)
    if player ~= localPlayer then
        player.CharacterAdded:Connect(function(character)
            task.wait(0.5)
            makeHeadSpherical(character)
            nameTags[character] = createNameTag(character, player.Name)
            chatBubbles[character] = createChatBubble()
        end)

        player.Chatted:Connect(function(msg)
            local character = player.Character
            if character then
                local bubble = chatBubbles[character]
                if bubble then
                    bubble.Text = msg
                    bubble.Visible = true
                    task.delay(5 + #msg / 20, function() bubble.Visible = false end)
                end
            end
        end)

        if player.Character then
            task.wait(0.5)
            makeHeadSpherical(player.Character)
            nameTags[player.Character] = createNameTag(player.Character, player.Name)
            chatBubbles[player.Character] = createChatBubble()
        end
    else
        localPlayer.CharacterAdded:Connect(function(character)
            task.wait(0.5)
            makeHeadSpherical(character)
        end)
        if localPlayer.Character then
            task.wait(0.5)
            makeHeadSpherical(localPlayer.Character)
        end
    end
end

Players.PlayerAdded:Connect(setupPlayer)
for _, p in ipairs(Players:GetPlayers()) do setupPlayer(p) end

workspace.DescendantAdded:Connect(function(d)
    if d:IsA("Model") and not Players:GetPlayerFromCharacter(d) then
        local hum = d:FindFirstChildOfClass("Humanoid")
        if hum then
            makeHeadSpherical(d)
            nameTags[d] = createNameTag(d, d.Name)
        end
    end
end)

UserInputService.InputBegan:Connect(function(input, processed)
    if not processed and input.KeyCode == Enum.KeyCode.Q then
        menuOpen = not menuOpen
        ConfigFrame.Visible = menuOpen
        promptText.Visible = not menuOpen
        scrollPromptText.Visible = menuOpen

        if menuOpen then
            gridSizeTextBox.Text = tostring(GRID_SIZE)
            screenScaleTextBox.Text = tostring(SCREEN_SCALE)
        else
            GRID_SIZE = math.clamp(math.floor(tonumber(gridSizeTextBox.Text) or GRID_SIZE), 1, 150)
            SCREEN_SCALE = math.clamp(tonumber(screenScaleTextBox.Text) or SCREEN_SCALE, 0.1, 1)

            mainFrame.Size = UDim2.new(SCREEN_SCALE, 0, SCREEN_SCALE, 0)
            mainFrame.Position = UDim2.new(0.5 - SCREEN_SCALE / 2, 0, 0.5 - SCREEN_SCALE / 2, 0)
            initializeGrid()
        end
    end
end)

initializeGrid()
