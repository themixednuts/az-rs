local function accumulator(initial)
  local value = initial
  return function(delta)
    value = value + delta
    return value
  end
end

local function firstAndLast(...)
  local values = { ... }
  return values[1], values[#values]
end

local add = accumulator(10)
print(add(2), add(4))
print(firstAndLast("north", "center", "south"))

