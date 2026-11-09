-- T8: closure-return specialization, multi-return destructure, upvalues
local function makeBar(fill)
    local maxT = 0
    local function set(value)
        if maxT <= 0 then maxT = value end
        fill.Size = UDim2.new(0, value, 0, 0)
    end
    return set
end
local setA = makeBar(workspace:WaitForChild("Part"))
setA(5)
setA(9)
