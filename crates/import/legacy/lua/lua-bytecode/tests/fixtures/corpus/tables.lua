local Counter = {
  values = {},
  total = 0,
}

function Counter:add(name, amount)
  local previous = self.values[name] or 0
  self.values[name] = previous + amount
  self.total = self.total + amount
  return self.values[name]
end

function Counter:largest()
  local selectedName = nil
  local selectedValue = nil
  for name, value in pairs(self.values) do
    if selectedValue == nil or value > selectedValue then
      selectedName = name
      selectedValue = value
    end
  end
  return selectedName, selectedValue
end

Counter:add("alpha", 2)
Counter:add("beta", 5)
print(Counter:largest())

