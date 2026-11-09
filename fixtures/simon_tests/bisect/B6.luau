-- B6: elseif mode machine + or in condition
local mode = "hide"
local newMode = "intermission"
if newMode == "hide" or newMode == false then
    mode = "hide"
elseif newMode == "notenough" then
    mode = "notenough"
elseif newMode == "intermission" or newMode == true then
    mode = "intermission"
elseif newMode == "ingame" then
    mode = "ingame"
end
print("b6 ok: " .. mode)
