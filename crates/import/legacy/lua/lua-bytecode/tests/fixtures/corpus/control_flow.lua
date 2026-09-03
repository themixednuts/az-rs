local function classify(value)
  if value < 0 then
    return "negative"
  elseif value == 0 then
    return "zero"
  end

  local total = 0
  for index = 1, value do
    total = total + index
  end
  while total > 20 do
    total = total - 3
  end
  repeat
    total = total + 1
  until total >= 10
  return "positive", total
end

for _, value in ipairs({ -1, 0, 7 }) do
  print(classify(value))
end

