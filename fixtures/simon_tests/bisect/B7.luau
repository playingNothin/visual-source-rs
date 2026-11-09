-- B7: string.gsub (GSubString - never imported before)
local box = workspace:WaitForChild("Box")
local text = string.gsub(box.Text, "^%s*(.-)%s*$", "%1")
print("b7 ok: " .. text)
