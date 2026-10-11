-- Unified `debug` global: overlay settings plus the standard Luau debug library.
function start()
    assert(debug.enabled == false, "overlay starts hidden")
    debug.enabled = true
    assert(debug.enabled == true)
    assert(debug.toggle() == false)
    debug.hotkey = false
    assert(debug.hotkey == false)
    debug.show_physics = true
    assert(debug.show_physics == true)
    debug.add_line("hello", Color.RED)
    debug.clear_lines()
    assert(type(debug.fps) == "number")
    assert(type(debug.avg_frame_ms()) == "number")
    -- standard library still reachable
    assert(type(debug.traceback()) == "string", "debug.traceback kept")
    assert(type(debug.info) == "function", "debug.info kept")
    -- read-only / unknown fields are errors, not silent writes
    assert(not pcall(function() debug.fps = 1 end), "fps is read-only")
    assert(not pcall(function() debug.nope = 1 end), "unknown field rejected")
    assert(not pcall(function() debug.enabled = "yes" end), "type checked")
    quit()
end
