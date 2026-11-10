-- T1: service refs, property paths, WaitForChild chains
local Players = game:GetService("Players")
local player = Players.LocalPlayer
local simon = workspace:WaitForChild("SimonSays")
local remotes = simon:WaitForChild("Remotes")
local label = remotes:WaitForChild("ZoneHint")
print(label.Name)
