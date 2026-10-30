-- Balatro Meta Watch: the companion of the real-game checks run by `make watch` (Balatro Meta project).
--
-- The checks play runs in a hidden copy of Balatro through BalatroBot and compare the game with the
-- engine after every move. While they run (a fresh control file for this game's BalatroBot port), this
-- mod:
--   * reports whether the game is still playing out the last move (queued animations), so the checks
--     compare only once the move is over;
--   * in the menu, makes sure BalatroBot can start the next run (see ensure_viewed_back);
--   * if asked, blocks mouse and keyboard (with a small badge) and sets the game speed.
-- It changes nothing in the rules. It talks through files next to the game's saves (love.filesystem):
-- balatro_meta/<BalatroBot port>/control.txt in, state.txt out, so a Balatro on another port (one you
-- play yourself) is never affected. Without a fresh control file it does nothing but report its state
-- once a second.

local VERSION = "0.4.0"
local PORT = tonumber(os.getenv("BALATROBOT_PORT")) or 12346
local DIR = "balatro_meta/" .. PORT
local CONTROL = DIR .. "/control.txt"
local STATE = DIR .. "/state.txt"
local STALE = 6 -- seconds without a new control stamp before the checks count as gone

local M = {
    version = VERSION,
    port = PORT,
    dir = DIR,
    control = {},
    stamp = nil,
    fresh_at = -1e9,
    last_read = -1e9,
    last_state = -1e9,
    saved = nil,
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

-- The speed asked for, while the checks run; the game's own settings come back when they stop.
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

-- BalatroBot's `start` sets the deck on the run-setup screen's deck (G.GAME.viewed_back). Steamodded's
-- run-select screen doesn't make that deck when there is no saved run to continue, which is always the
-- case once a run has ended, so every `start` after a lost run would fail. While the checks run, make
-- the default one in the menu, as the game's own screen does; BalatroBot then sets the deck it asks for.
local function ensure_viewed_back()
    if G and G.STATES and G.STATE == G.STATES.MENU and G.GAME and G.GAME.viewed_back == nil
        and type(Back) == "table" and G.P_CENTERS and G.P_CENTERS.b_red then
        G.GAME.viewed_back = Back(G.P_CENTERS.b_red)
    end
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
        "port=" .. PORT,
        "time=" .. string.format("%.3f", now()),
        "busy=" .. busy(),
        "state=" .. state_name(),
        "complete=" .. ((G and G.STATE_COMPLETE) and "1" or "0"),
        "speed=" .. tostring(G and G.SETTINGS and G.SETTINGS.GAMESPEED or ""),
        "locked=" .. (locked() and "1" or "0"),
        "control=" .. tostring(M.stamp or ""),
    }
    if M.error then
        lines[#lines + 1] = "error=" .. tostring(M.error):gsub("[\r\n]", " ")
    end
    lines[#lines + 1] = "end=1" -- the checks ignore a report they read half-written
    love.filesystem.write(STATE, table.concat(lines, "\n") .. "\n")
end

-- Every frame, in love.update (which runs in BalatroBot's headless mode too).
function M.update()
    local t = now()
    if active() then
        ensure_viewed_back()
    end
    if t - M.last_read >= 0.1 then
        M.last_read = t
        read_control()
        apply_speed()
    end
    if t - M.last_state >= (active() and 0.05 or 1.0) then
        M.last_state = t
        write_state()
    end
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

function M.draw()
    if locked() then
        draw_badge()
    end
end

pcall(love.filesystem.createDirectory, DIR)

-- Input: swallowed while locked.
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

local game_update = love.update
love.update = function(...)
    if game_update then
        game_update(...)
    end
    local ok, err = pcall(M.update)
    if not ok then
        M.error = err
    end
end

local game_draw = love.draw
love.draw = function(...)
    if game_draw then
        game_draw(...)
    end
    local ok, err = pcall(M.draw)
    if not ok then
        M.error = err
    end
end
