-- T2b: literal vs computed UDim2
local part = workspace:WaitForChild("Part")
part.Size = UDim2.new(0, 180, 0, 3)
local w = 100
part.Position = UDim2.new(0.5, w, 1, -14)
print("sized")
