local counter = { value = 0 }

function counter.bump()
    counter.value += 1
end

return counter
