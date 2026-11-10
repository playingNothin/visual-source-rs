-- B8: for-iterations over GetPlayers + chained property reads
local Players = game:GetService("Players")
local TEAM = BrickColor.new(37)
local n = 0
for _, plr in ipairs(Players:GetPlayers()) do
    if plr.TeamColor ~= TEAM then
        n = n + 1
    end
end
local label = Players.LocalPlayer
print("b8 ok: " .. n)
