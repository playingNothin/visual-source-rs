-- T5: tuple-output event (first live TablePack path)
local re = workspace:WaitForChild("RemoteEvent")
re.OnClientEvent:Connect(function(show, extra)
    if show then
        print("shown: " .. tostring(extra))
    end
end)
