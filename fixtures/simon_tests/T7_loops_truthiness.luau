-- T7: while true, break, truthiness on a bare var (PREDICTED: empty Value 2)
local i = 0
while true do
    i = i + 1
    wait(0.5)
    if i >= 3 then
        break
    end
end
local done = false
if done then
    print("never")
end
print("loop done: " .. i)
