-- T2: constructor literals into variables (PREDICTED: missing producers)
local WHITE = Color3.new(1, 1, 1)
local RED = Color3.fromRGB(255, 60, 60)
local TEAM = BrickColor.new(21)
local frame = Instance.new("Frame")
frame.BackgroundColor3 = RED
frame.BorderColor3 = WHITE
frame.Parent = workspace
print("colors: " .. TEAM.Name)
