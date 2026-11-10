-- T9: mode machine - event + main loop drive the same function
local mode = "hide"
local status = workspace:WaitForChild("Status")
local function refresh()
    if mode == "notenough" then
        status.Text = "Not enough"
    elseif mode == "intermission" then
        status.Text = "Intermission"
    else
        status.Text = "Waiting"
    end
end
refresh()
status.Changed:Connect(function()
    refresh()
end)
while true do
    wait(1)
    refresh()
end
