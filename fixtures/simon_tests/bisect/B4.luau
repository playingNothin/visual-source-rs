-- B4: named + tuple events (proven: T4, T5)
local box = workspace:WaitForChild("Box")
box.Changed:Connect(function(prop)
    if prop == "Text" then
        print("changed")
    end
end)
local re = workspace:WaitForChild("RemoteEvent")
re.OnClientEvent:Connect(function(show, extra)
    if show then
        print("shown: " .. tostring(extra))
    end
end)
print("b4 ok")
