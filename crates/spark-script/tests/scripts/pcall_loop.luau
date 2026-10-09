-- A pcall around an endless loop must not swallow the timeout.
function update(dt)
    while true do
        pcall(function()
            while true do
            end
        end)
    end
end
