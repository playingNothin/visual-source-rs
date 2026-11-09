-- T6: length, concat, math.min, tostring
local Players = game:GetService("Players")
local n = #Players:GetPlayers()
local status = "Players: " .. n
local v = math.min(n, 10)
print(status .. " / " .. tostring(v))
