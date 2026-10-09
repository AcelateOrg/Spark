-- Contract test for the scheduler + events. Runs headless at a fixed 1/60 dt (see tests/flow.rs).
-- Calls quit() when every check passed; any error fails the test.

local log = {}
local function push(x) table.insert(log, x) end
local function expect(cond, msg) if not cond then error("CHECK FAILED: " .. msg, 2) end end
local function near(a, b, eps) return math.abs(a - b) <= (eps or 1e-3) end

local frame_log = {}
local record_frame = 0 -- which frame to record the phase order for

function fixed_update(dt)
    expect(near(dt, 1 / 60, 1e-6), "fixed dt is 1/60")
    if time.frame == record_frame then table.insert(frame_log, "fixed") end
end

function update(dt)
    if time.frame == record_frame then
        table.insert(frame_log, "update")
        emit_later("phase_deferred")
        next_frame(function() end)
    end
end

function late_update(dt)
    if time.frame == record_frame then table.insert(frame_log, "late") end
end

local function check_phases()
    record_frame = time.frame + 1
    on("phase_deferred", function() table.insert(frame_log, "deferred") end)
    after(0, function() table.insert(frame_log, "timer") end)
    wait_frames(2)
    local got = table.concat(frame_log, ",")
    expect(got == "fixed,update,timer,deferred,late", "frame phase order, got " .. got)
end

local function check_timer_order()
    log = {}
    after(0.1, function() push("b") end)
    after(0.05, function() push("a") end)
    after(0.1, function() push("c") end)
    local t = after(0.1, function() push("cancelled") end)
    t:cancel()
    expect(not t.active, "cancelled timer is inactive")
    wait(0.2)
    expect(table.concat(log) == "abc", "timers fire by due time then schedule order, got " .. table.concat(log))
end

local function check_every()
    local counts = {}
    local timer = every(0.1, function(n) table.insert(counts, n) end, { times = 3 })
    wait(0.5)
    expect(#counts == 3 and counts[3] == 3, "every with times=3 fires 3 times")
    expect(not timer.active, "finished every timer is inactive")

    -- catch-up: a 0.5 s frame (via time.scale) must still give one tick per interval
    local n = 0
    every(0.05, function() n += 1 end, { times = 10 })
    time.scale = 30 -- next frame dt = 0.5 s
    wait_frames(1)
    time.scale = 1
    wait_frames(1)
    expect(n == 10, "every catches up in a long frame, got " .. n)
end

local function check_pause_and_scale()
    local fired = false
    local t = after(0.1, function() fired = true end)
    t:pause()
    wait(0.3)
    expect(not fired and t.paused, "paused timer does not fire")
    t:resume()
    wait(0.15)
    expect(fired, "resumed timer fires")

    local scaled = false
    after(0.05, function() scaled = true end)
    time.scale = 0
    for _ = 1, 10 do wait_frames(1) end
    expect(not scaled, "time.scale = 0 pauses timers")
    time.scale = 1
    wait(0.1)
    expect(scaled, "timers continue after unpausing")
end

local function check_wait_values()
    local elapsed = wait(0.25)
    expect(near(elapsed, 0.25, 1 / 60 + 1e-6), "wait returns elapsed time, got " .. elapsed)

    local f0 = time.frame
    wait_frames(3)
    expect(time.frame == f0 + 3, "wait_frames(3) waits exactly 3 frames")

    local flag = false
    after(0.1, function() flag = true end)
    expect(wait_until(function() return flag end) == true, "wait_until returns true")
    expect(wait_until(function() return false end, 0.1) == nil, "wait_until timeout returns nil")
end

local function check_wait_event()
    after(0.05, function() emit("door", "red", 3) end)
    local color, n = wait_event("door")
    expect(color == "red" and n == 3, "wait_event returns event args")

    after(0.05, function() emit("ping") end)
    expect(wait_event("ping") == true, "argless event resumes with true")

    expect(wait_event("never", 0.1) == nil, "wait_event timeout returns nil")
    -- the timed-out wait must not resume the task later
    emit("never", 1)
end

local function check_listeners()
    log = {}
    local a = on("hit", function(x) push("a" .. x) end)
    on("hit", function(x) push("high" .. x) end, { priority = 10 })
    once("hit", function(x) push("once" .. x) end)
    emit("hit", 1)
    emit("hit", 2)
    expect(table.concat(log, " ") == "high1 a1 once1 high2 a2", "priority, registration order, once: " .. table.concat(log, " "))

    a:off()
    expect(not a.active, "off() deactivates")
    log = {}
    on("hit", function() push("stop") return true end, { priority = 5 })
    local consumed = emit("hit", 3)
    expect(consumed and table.concat(log, " ") == "high3 stop", "returning true consumes the event")
    off("hit")
    expect(emit("hit", 4) == false, "off(name) removes all listeners")

    -- snapshot: a listener added during emit does not receive that emit
    local late = 0
    on("snap", function() on("snap", function() late += 1 end) end)
    emit("snap")
    expect(late == 0, "listener added during emit is not called for it")
    off("snap")

    -- infinite recursion is caught
    on("loop", function() emit("loop") end)
    local ok, err = pcall(emit, "loop")
    expect(not ok and string.find(tostring(err), "infinite loop"), "recursive emit is an error")
    off("loop")
end

local function check_owners()
    local box = spawn(Mesh.cube(), "red", { name = "owned" })
    local timer_fired, heard, obj_heard = false, 0, nil
    after(0.1, function() timer_fired = true end, box)
    on("boom", function() heard += 1 end, { owner = box })
    box:on("hurt", function(dmg) obj_heard = dmg end)
    box:emit("hurt", 5)
    expect(obj_heard == 5, "object events reach object listeners")
    expect(emit("hurt", 1) == false, "object events are separate from global ones")
    emit("boom")
    box:destroy()
    emit("boom")
    wait(0.2)
    expect(heard == 1 and not timer_fired, "destroying the owner removes its listeners and timers")
end

local function check_tasks()
    local steps = 0
    local t = task(function()
        while true do
            steps += 1
            wait(0.05)
        end
    end)
    expect(t.status == "waiting" and t.alive, "task runs until its first wait")
    wait(0.12)
    t:cancel()
    local s = steps
    wait(0.2)
    expect(steps == s and t.status == "cancelled", "cancelled task never resumes")

    local done = task(function(a, b) return a + b end, 1, 2)
    expect(done.status == "done", "task without waits finishes immediately")

    local ok, err = pcall(task, function() error("boom") end)
    expect(not ok and string.find(tostring(err), "boom"), "error in a task propagates")

    -- timer callbacks are plain calls (not tasks): wait() there must fail clearly
    local ok2, err2 = true, nil
    after(0, function() ok2, err2 = pcall(wait, 1) end)
    wait_frames(2)
    expect(not ok2 and string.find(tostring(err2), "only works inside task"), "wait outside a task is a clear error")
end

function start()
    -- start() is itself a task, so it can wait.
    check_phases()
    check_timer_order()
    check_every()
    check_pause_and_scale()
    check_wait_values()
    check_wait_event()
    check_listeners()
    check_owners()
    check_tasks()
    print("flow contract: all checks passed at frame " .. time.frame)
    quit()
end
