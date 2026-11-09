-- T3: object path as a property value
local gui = workspace:WaitForChild("A")
local root = gui:WaitForChild("Panel")
root.Parent = gui
print("reparented")
