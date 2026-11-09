-- T4: named-output events, direct params
local box = Instance.new("TextBox")
box.Parent = workspace
box.Changed:Connect(function(prop)
    if prop == "Text" then
        print("changed: " .. prop)
    end
end)
workspace:WaitForChild("Part").Touched:Connect(function(otherPart)
    print(otherPart.Name)
end)
