local function range(limit)
  local current = 0
  return function()
    current = current + 1
    if current <= limit then
      return current, current * current
    end
  end
end

local squares = {}
for index, square in range(5) do
  squares[index] = square
end

for index = #squares, 1, -1 do
  print(index, squares[index])
end

