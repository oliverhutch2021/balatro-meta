-- Balatro Meta Watch: the companion of `make watch` (Balatro Meta project).
--
-- While the viewer runs, this mod:
--   * blocks mouse and keyboard, so nothing disturbs the bot's moves (a small badge says so);
--   * sets the game speed the viewer asks for;
--   * saves a small picture of the game a few times a second, which the viewer shows;
--   * reports whether the game is still playing out the last move (queued animations), so the
--     viewer waits before sending the next one.
-- It changes nothing in the rules. It talks to the viewer through files next to the game's saves
-- (love.filesystem): balatro_meta/control.txt in; balatro_meta/state.txt and frame_<n>.png out.
-- If the viewer stops (no fresh control file for a few seconds), input comes back, the speed goes
-- back to what it was, and capturing stops.

local VERSION = "0.2.0"
local DIR = "balatro_meta"
local CONTROL = DIR .. "/control.txt"
local STATE = DIR .. "/state.txt"
local STALE = 6 -- seconds without a new control stamp before the viewer counts as gone

local M = {
    version = VERSION,
    control = {},
    stamp = nil,
    fresh_at = -1e9,
    last_read = -1e9,
    last_frame = -1e9,
    last_state = -1e9,
    seq = 0,
    slot = 0,
    busy_encoding = false,
    small = nil,
    saved = nil,
    frame = nil,
    error = nil,
}
BALATRO_META_WATCH = M -- one global, for debugging

local function now()
    return love.timer.getTime()
end

local function parse(text)
    local t = {}
    for k, v in text:gmatch("([%w_]+)=([^\r\n]*)") do
        t[k] = v
    end
    return t
end

local function active()
    return now() - M.fresh_at < STALE
end

local function locked()
    return active() and M.control.lock == "1"
end
M.locked = locked
M.active = active

local function read_control()
    local ok, text = pcall(love.filesystem.read, CONTROL)
    if not ok or type(text) ~= "string" then
        return
    end
    local c = parse(text)
    if c.n and c.n ~= M.stamp then
        M.stamp = c.n
        M.fresh_at = now()
    end
    M.control = c
end

-- The viewer's speed while it runs; the game's own settings come back when it stops.
local function apply_speed()
    if not (G and G.SETTINGS) then
        return
    end
    local speed = tonumber(M.control.speed)
    if active() and speed then
        if not M.saved then
            M.saved = { speed = G.SETTINGS.GAMESPEED, cap = G.FPS_CAP }
        end
        G.SETTINGS.GAMESPEED = speed
        if M.control.uncap == "1" then
            G.FPS_CAP = nil
        else
            G.FPS_CAP = M.saved.cap
        end
    elseif M.saved then
        G.SETTINGS.GAMESPEED = M.saved.speed
        G.FPS_CAP = M.saved.cap
        M.saved = nil
    end
end

-- Blocking events still queued: the game is still playing out the last move.
local function busy()
    local q = G and G.E_MANAGER and G.E_MANAGER.queues and G.E_MANAGER.queues.base
    if type(q) ~= "table" then
        return -1
    end
    local n = 0
    for _, ev in ipairs(q) do
        if ev.blocking ~= false and not ev.complete then
            n = n + 1
        end
    end
    return n
end

local function state_name()
    if not (G and G.STATES and G.STATE) then
        return ""
    end
    for k, v in pairs(G.STATES) do
        if v == G.STATE then
            return k
        end
    end
    return tostring(G.STATE)
end

local function write_state()
    local lines = {
        "version=" .. VERSION,
        "seq=" .. M.seq,
        "time=" .. string.format("%.3f", now()),
        "busy=" .. busy(),
        "state=" .. state_name(),
        "complete=" .. ((G and G.STATE_COMPLETE) and "1" or "0"),
        "speed=" .. tostring(G and G.SETTINGS and G.SETTINGS.GAMESPEED or ""),
        "locked=" .. (locked() and "1" or "0"),
        "control=" .. tostring(M.stamp or ""),
    }
    if M.frame then
        lines[#lines + 1] = "frame=" .. M.frame.name
        lines[#lines + 1] = "width=" .. M.frame.w
        lines[#lines + 1] = "height=" .. M.frame.h
    end
    if M.error then
        lines[#lines + 1] = "error=" .. tostring(M.error):gsub("[\r\n]", " ")
    end
    love.filesystem.write(STATE, table.concat(lines, "\n") .. "\n")
end

-- PNG encoding and writing happen on a LOVE thread, so the game never waits for them.
local ENCODER = [[
require("love.image")
require("love.filesystem")
local inbox = love.thread.getChannel("bmwatch_frames")
local outbox = love.thread.getChannel("bmwatch_written")
while true do
    local job = inbox:demand()
    if job == "stop" then
        return
    end
    local ok, err = pcall(function()
        job.data:encode("png", job.path)
    end)
    outbox:push({ name = job.name, w = job.w, h = job.h, ok = ok, err = err and tostring(err) or nil })
end
]]

local function encoder()
    if M.thread == nil then
        M.thread = love.thread.newThread(ENCODER)
        M.thread:start()
        M.inbox = love.thread.getChannel("bmwatch_frames")
        M.outbox = love.thread.getChannel("bmwatch_written")
    end
    return M.thread
end

-- Frames the encoder has finished writing.
local function collect()
    if not M.outbox then
        return
    end
    while true do
        local done = M.outbox:pop()
        if not done then
            break
        end
        M.busy_encoding = false
        if done.ok then
            M.seq = M.seq + 1
            M.frame = { name = done.name, w = done.w, h = done.h }
        else
            M.error = done.err
        end
    end
    if M.thread and M.thread:getError() then
        M.error = M.thread:getError()
    end
end

local function hand_over(data, w, h)
    M.slot = (M.slot + 1) % 3
    local name = "frame_" .. M.slot .. ".png"
    encoder()
    M.busy_encoding = true
    M.inbox:push({ data = data, path = DIR .. "/" .. name, name = name, w = w, h = h })
end

-- If the game's canvas can't be drawn from, fall back to LOVE's screenshot (full size, fewer per second).
local function capture_screen()
    M.busy_encoding = true
    love.graphics.captureScreenshot(function(data)
        local w, h = data:getDimensions()
        M.busy_encoding = false
        hand_over(data, w, h)
    end)
end

-- A small copy of the game's own canvas (before screen effects), handed to the encoder.
local function capture()
    if M.busy_encoding then
        return
    end
    if M.use_screenshot then
        return capture_screen()
    end
    local src = G and G.CANVAS
    if not src then
        return
    end
    local sw, sh = src:getDimensions()
    local w = math.max(64, math.min(tonumber(M.control.width) or 960, sw))
    local h = math.max(36, math.floor(sh * w / sw + 0.5))
    if not M.small or M.small:getWidth() ~= w or M.small:getHeight() ~= h then
        M.small = love.graphics.newCanvas(w, h)
    end
    love.graphics.push("all")
    love.graphics.setCanvas(M.small)
    love.graphics.origin()
    love.graphics.setShader()
    love.graphics.setBlendMode("alpha", "alphamultiply")
    love.graphics.clear(0, 0, 0, 1)
    love.graphics.setColor(1, 1, 1, 1)
    local fmin, fmag = src:getFilter()
    src:setFilter("linear", "linear")
    love.graphics.draw(src, 0, 0, 0, w / sw, h / sh)
    src:setFilter(fmin, fmag)
    love.graphics.pop()
    hand_over(M.small:newImageData(), w, h)
end

local function draw_badge()
    local text = "Balatro Meta is playing - mouse and keyboard are off"
    local font = love.graphics.getFont()
    local w, h = font:getWidth(text), font:getHeight()
    local sw = love.graphics.getWidth()
    love.graphics.push("all")
    love.graphics.setCanvas()
    love.graphics.origin()
    love.graphics.setShader()
    love.graphics.setColor(0, 0, 0, 0.7)
    love.graphics.rectangle("fill", sw - w - 24, 8, w + 16, h + 10, 6, 6)
    love.graphics.setColor(1, 1, 1, 0.95)
    love.graphics.print(text, sw - w - 16, 13)
    love.graphics.pop()
end

function M.tick()
    local t = now()
    collect()
    if t - M.last_read > 0.25 then
        M.last_read = t
        read_control()
        apply_speed()
    end
    if active() and M.control.frames == "1" then
        local every = tonumber(M.control.every) or 0.2
        if M.use_screenshot then
            every = math.max(every, 0.5)
        end
        if t - M.last_frame >= every then
            M.last_frame = t
            local ok, err = pcall(capture)
            if not ok then
                M.error = err
                if not M.use_screenshot then
                    M.use_screenshot = true -- the canvas wasn't readable: use screenshots from now on
                    M.busy_encoding = false
                    love.graphics.setCanvas()
                end
            end
        end
    end
    if t - M.last_state >= (active() and 0.05 or 1.0) then
        M.last_state = t
        write_state()
    end
    if locked() then
        draw_badge()
    end
end

pcall(love.filesystem.createDirectory, DIR)

-- Input: swallowed while the viewer has the game locked.
local INPUT = {
    "mousepressed", "mousereleased", "mousemoved", "wheelmoved", "keypressed", "keyreleased", "textinput",
    "touchpressed", "touchreleased", "touchmoved", "joystickpressed", "joystickreleased", "joystickaxis",
    "joystickhat", "gamepadpressed", "gamepadreleased", "gamepadaxis",
}
for _, name in ipairs(INPUT) do
    local original = love[name]
    if original then
        love[name] = function(...)
            if locked() then
                return
            end
            return original(...)
        end
    end
end
-- The game also reads the mouse directly every frame (hover); keep it off the table while locked.
if love.mouse then
    local getPosition, getX, getY, isDown =
        love.mouse.getPosition, love.mouse.getX, love.mouse.getY, love.mouse.isDown
    love.mouse.getPosition = function(...)
        if locked() then
            return -200, -200
        end
        return getPosition(...)
    end
    if getX then
        love.mouse.getX = function(...)
            if locked() then
                return -200
            end
            return getX(...)
        end
    end
    if getY then
        love.mouse.getY = function(...)
            if locked() then
                return -200
            end
            return getY(...)
        end
    end
    if isDown then
        love.mouse.isDown = function(...)
            if locked() then
                return false
            end
            return isDown(...)
        end
    end
end

local game_draw = love.draw
love.draw = function(...)
    if game_draw then
        game_draw(...)
    end
    local ok, err = pcall(M.tick)
    if not ok then
        M.error = err
    end
end
