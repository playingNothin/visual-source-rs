-- B3: create + property forests (proven: T2, T3)
local root = Instance.new("Frame")
root.Name = "WordInput"
root.AnchorPoint = Vector2.new(0.5, 1)
root.Size = UDim2.new(0, 360, 0, 54)
root.BackgroundColor3 = Color3.fromRGB(30, 26, 22)
root.Visible = false
root.Parent = workspace
local box = Instance.new("TextBox")
box.Font = Enum.Font.SourceSans
box.TextSize = 18
box.ClearTextOnFocus = false
box.Parent = root
print("b3 ok")
