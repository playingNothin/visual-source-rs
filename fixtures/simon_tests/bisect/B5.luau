-- B5: closures + destructured 2-return + and/or in value position
local function barController(fill)
    local maxT = 0
    local function set(value)
        if maxT <= 0 then maxT = value end
        fill.Size = UDim2.new(0, value, 0, 0)
    end
    local function reset()
        maxT = 0
        fill.Size = UDim2.new(0, 0, 0, 0)
    end
    return set, reset
end
local setA, resetA = barController(workspace:WaitForChild("Part"))
local show = true
local toY = show and 10 or -130
print("b5 ok: " .. tostring(toY))
