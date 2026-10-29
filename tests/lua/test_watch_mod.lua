-- The companion mod against a stand-in for LOVE and the game: run with `luajit tests/lua/test_watch_mod.lua`.
local files = {}
local clock = 100.0
local calls = { pressed = 0, drawn = 0 }

local function data(bytes)
    return { bytes = bytes, encode = function(self, fmt, path) files[path] = "PNG:" .. #self.bytes end }
end
-- Channels and a "thread" that does its work as soon as a job is pushed.
local channels = {}
local function channel(name)
    channels[name] = channels[name] or { items = {} }
    local c = channels[name]
    c.push = function(self, v)
        if name == "bmwatch_frames" and v ~= "stop" then
            v.data:encode("png", v.path)
            channel("bmwatch_written"):push({ name = v.name, w = v.w, h = v.h, ok = true })
        else
            table.insert(self.items, v)
        end
    end
    c.pop = function(self) return table.remove(self.items, 1) end
    c.demand = c.pop
    return c
end
local canvas_mt = {}
canvas_mt.__index = canvas_mt
function canvas_mt:getDimensions() return self.w, self.h end
function canvas_mt:getWidth() return self.w end
function canvas_mt:getHeight() return self.h end
function canvas_mt:getFilter() return self.fmin or "nearest", self.fmag or "nearest" end
function canvas_mt:setFilter(a, b) self.fmin, self.fmag = a, b end
function canvas_mt:newImageData() return data(string.rep("\0", self.w * self.h * 4)) end
local function canvas(w, h) return setmetatable({ w = w, h = h }, canvas_mt) end

love = {
    timer = { getTime = function() return clock end },
    filesystem = {
        read = function(name) return files[name] end,
        write = function(name, d) files[name] = type(d) == "table" and d.bytes or d; return true end,
        createDirectory = function() return true end,
    },
    graphics = {
        push = function() end, pop = function() end, setCanvas = function() end, origin = function() end,
        setShader = function() end, setBlendMode = function() end, clear = function() end,
        setColor = function() end, draw = function() end, rectangle = function() end, print = function() end,
        newCanvas = canvas, getWidth = function() return 1920 end,
        getFont = function() return { getWidth = function() return 100 end, getHeight = function() return 12 end } end,
    },
    mouse = { getPosition = function() return 500, 400 end, isDown = function() return true end },
    thread = {
        newThread = function() return { start = function() end, getError = function() return nil end } end,
        getChannel = channel,
    },
    mousepressed = function() calls.pressed = calls.pressed + 1 end,
    draw = function() calls.drawn = calls.drawn + 1 end,
}
G = {
    CANVAS = canvas(1920, 1080), SETTINGS = { GAMESPEED = 4 }, FPS_CAP = 60,
    STATES = { SHOP = 5, ROUND_EVAL = 8 }, STATE = 5, STATE_COMPLETE = true,
    E_MANAGER = { queues = { base = {} } },
}

dofile("mods/balatro_meta_watch/main.lua")
local M = BALATRO_META_WATCH
local function frame(dt) clock = clock + (dt or 0.3); love.draw() end
local function state()
    local t = {}
    for k, v in (files["balatro_meta/state.txt"] or ""):gmatch("([%w_]+)=([^\n]*)") do t[k] = v end
    return t
end

-- No viewer yet: input passes, no frames, the speed is the game's own.
frame()
love.mousepressed()
assert(calls.pressed == 1 and calls.drawn == 1)
assert(select(1, love.mouse.getPosition()) == 500)
assert(state().locked == "0" and state().seq == "0" and G.SETTINGS.GAMESPEED == 4)

-- The viewer locks, sets speed 10 with the frame cap off, and asks for frames.
files["balatro_meta/control.txt"] = "n=1\nlock=1\nspeed=10\nuncap=1\nframes=1\nwidth=960\nevery=0.2\n"
frame()
love.mousepressed()
assert(calls.pressed == 1, "a click while locked must be swallowed")
assert(select(1, love.mouse.getPosition()) == -200 and love.mouse.isDown(1) == false)
assert(G.SETTINGS.GAMESPEED == 10 and G.FPS_CAP == nil)
frame()
local s = state()
assert(s.locked == "1" and tonumber(s.seq) >= 1 and s.width == "960" and s.height == "540", s.width)
assert(files["balatro_meta/" .. s.frame] == "PNG:" .. 960 * 540 * 4, tostring(files["balatro_meta/" .. s.frame]))
assert(s.state == "SHOP" and s.busy == "0" and s.version == "0.2.0")

-- Busy while blocking events are queued; non-blocking ones don't count.
G.E_MANAGER.queues.base = { { blocking = true }, { blocking = false }, {} }
frame()
assert(state().busy == "2")
G.E_MANAGER.queues.base = {}

-- The viewer goes away: after the stale time input, speed and frame cap come back.
for _ = 1, 30 do frame(0.3) end
love.mousepressed()
assert(calls.pressed == 2, "input must come back once the viewer is gone")
assert(G.SETTINGS.GAMESPEED == 4 and G.FPS_CAP == 60)
assert(state().locked == "0")

-- A new stamp brings it back; frames rotate over three files.
files["balatro_meta/control.txt"] = "n=2\nlock=0\nspeed=2\nframes=1\nevery=0.2\n"
local seen = {}
for _ = 1, 6 do frame(0.25); seen[state().frame] = true end
local n = 0
for _ in pairs(seen) do n = n + 1 end
assert(n == 3, "frames rotate over three files")
assert(G.SETTINGS.GAMESPEED == 2 and G.FPS_CAP == 60 and state().locked == "0")
assert(not M.error, tostring(M.error))

-- A canvas that can't be drawn from: the mod falls back to screenshots.
love.graphics.draw = function() error("canvas not readable") end
local shots = 0
love.graphics.captureScreenshot = function(cb)
    shots = shots + 1
    local d = data(string.rep("\0", 16 * 9 * 4))
    d.getDimensions = function() return 16, 9 end
    cb(d)
end
for _ = 1, 6 do frame(0.3) end
assert(M.use_screenshot and shots >= 1, "falls back to screenshots")
assert(state().width == "16" and state().height == "9")
print("watch mod: all checks passed")
