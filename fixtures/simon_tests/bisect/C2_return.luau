-- C2: return-comma probe (N10 + X5 end-to-end)
local function two()
    return 1, 2
end
local a, b = two()
print(a)
print(b)
