-- `ui` widgets: return values, id rules, argument errors. (Mouse interaction is covered by Rust tests.)
-- Widgets are placed away from the origin: the headless mouse sits at (0, 0).
local volume, music = 0.5, true

function render()
    assert(ui.panel(100, 100, 300, 200) == false, "mouse is not over the panel")
    assert(ui.panel(100, 100, 300, 200, Color.BLACK, "#ffffff") == false)
    assert(ui.button("start", 110, 110, 200, 40, "Start") == false)
    assert(ui.button(1, 110, 160, 200, 40, "One") == false, "integer ids work")
    ui.label(110, 210, "Score: 1", 16, Color.WHITE)
    ui.label(110, 230, "defaults")
    ui.progress(110, 250, 200, 10, 0.3)
    volume = ui.slider("volume", 110, 270, 200, 20, volume, 0, 1)
    assert(volume == 0.5, "untouched slider returns the value")
    music = ui.checkbox("music", 320, 270, 20, music)
    assert(music == true, "untouched checkbox returns the state")
    assert(ui.hovered() == false)
    assert(not pcall(ui.button, nil, 0, 0, 10, 10, "x"), "id is required")
    assert(not pcall(ui.button, 1.5, 0, 0, 10, 10, "x"), "fractional ids are rejected")
    assert(not pcall(ui.image, 0, 0, 10, 10, 5), "image needs a texture")
    assert(ui.begin == nil and ui["end"] == nil, "the engine runs the UI frame")
    quit()
end
