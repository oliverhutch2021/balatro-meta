-- The companion mod against a stand-in for LOVE and the game: run with `luajit tests/lua/test_watch_mod.lua`.
local files = {}
local clock = 100.0
local calls = { pressed = 0, drawn = 0, updated = 0 }

local released = 0
local function data(bytes, w, h)
    return {
        bytes = bytes,
        getString = function(self) return self.bytes end,
        getSize = function(self) return #self.bytes end,
        getDimensions = function() return w, h end,
        encode = function(self, fmt, path) files[path] = "PNG:" .. #self.bytes end,
        release = function(self) self.released = true; released = released + 1 end,
    }
end
local canvas_mt = {}
canvas_mt.__index = canvas_mt
function canvas_mt:getDimensions() return self.w, self.h end
function canvas_mt:getWidth() return self.w end
function canvas_mt:getHeight() return self.h end
function canvas_mt:getFilter() return self.fmin or "nearest", self.fmag or "nearest" end
function canvas_mt:setFilter(a, b) self.fmin, self.fmag = a, b end
function canvas_mt:newImageData() return data(string.rep("\0", self.w * self.h * 4), self.w, self.h) end
local function canvas(w, h) return setmetatable({ w = w, h = h }, canvas_mt) end

-- Channels, and threads: the encoder does its job as soon as one is pushed; the streamer only records.
local channels = {}
local threads = {}
local function channel(name)
    if channels[name] then return channels[name] end
    local c = { items = {} }
    function c:push(v)
        if name == "bmwatch_encode" and v ~= "stop" then
            v.data:encode("png", v.path)
            channel("bmwatch_encoded"):push({ kind = v.kind, name = v.name, n = v.n, w = v.w, h = v.h, ok = true })
        else
            table.insert(self.items, v)
        end
    end
    function c:pop() return table.remove(self.items, 1) end
    function c:demand() return table.remove(self.items, 1) end
    function c:getCount() return #self.items end
    function c:clear() self.items = {} end
    channels[name] = c
    return c
end

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
        newThread = function(code)
            local t = { code = code, started = nil }
            function t:start(...) self.started = { ... } end
            function t:getError() return nil end
            threads[#threads + 1] = t
            return t
        end,
        getChannel = channel,
    },
    mousepressed = function() calls.pressed = calls.pressed + 1 end,
    update = function() calls.updated = calls.updated + 1 end,
    draw = function() calls.drawn = calls.drawn + 1 end,
}
G = {
    CANVAS = canvas(1920, 1080), SETTINGS = { GAMESPEED = 4 }, FPS_CAP = 60,
    STATES = { SHOP = 5, ROUND_EVAL = 8, MENU = 11 }, STATE = 5, STATE_COMPLETE = true,
    E_MANAGER = { queues = { base = {} } },
    GAME = {}, P_CENTERS = { b_red = { name = "Red Deck" } },
}
Back = setmetatable({}, { __call = function(_, center) return { name = center.name } end })

dofile("mods/balatro_meta_watch/main.lua")
local M = BALATRO_META_WATCH
local function frame(dt) clock = clock + (dt or 0.3); love.update(1 / 60); love.draw() end
local function state()
    local t = {}
    for k, v in (files["balatro_meta/state.txt"] or ""):gmatch("([%w_]+)=([^\n]*)") do t[k] = v end
    return t
end
local function streamer()
    for _, t in ipairs(threads) do
        if t.code == M.STREAMER then return t end
    end
end

-- The thread code compiles.
assert(loadstring(M.STREAMER), "streamer code compiles")
assert(loadstring(M.ENCODER), "encoder code compiles")

-- The BMP header for raw RGBA rows, top to bottom (checked against Python's struct).
local want = "424d7a100e00000000007a0000006c0000008002000098feffff010020000300000000100e00130b0000130b00000000000000000000ff00000000ff00000000ff00000000ff42475273" .. string.rep("00", 48)
local got = M.bmp_header(640, 360):gsub(".", function(c) return string.format("%02x", c:byte()) end)
assert(got == want, "bmp header:\n" .. got .. "\n" .. want)

-- No viewer yet: input passes, nothing streams, the speed is the game's own, the menu is untouched.
G.STATE = G.STATES.MENU
frame()
assert(G.GAME.viewed_back == nil)
G.STATE = 5
love.mousepressed()
assert(calls.pressed == 1 and calls.drawn == 1 and calls.updated == 1)
assert(select(1, love.mouse.getPosition()) == 500)
local s = state()
assert(s.locked == "0" and s.stream_status == "off" and s.stream_port == "13346" and G.SETTINGS.GAMESPEED == 4)

-- The viewer locks, sets 1x speed and asks for the stream.
files["balatro_meta/control.txt"] = "n=1\nlock=1\nspeed=1\nuncap=0\nstream=1\nwidth=640\nfps=30\n"
frame()
love.mousepressed()
assert(calls.pressed == 1, "a click while locked must be swallowed")
assert(select(1, love.mouse.getPosition()) == -200 and love.mouse.isDown(1) == false)
assert(G.SETTINGS.GAMESPEED == 1 and G.FPS_CAP == 60)
local st = streamer()
assert(st and st.started and st.started[1] == 13346, "the stream starts on BalatroBot's port + 1000")
assert(state().stream_status == "starting")
local frames_name, info_name = st.started[2], st.started[3]
assert(frames_name ~= info_name and frames_name:match("^bmwatch_stream_"), "this start's own channels")

-- Nobody watching yet: no pictures are made.
frame()
assert(channel(frames_name):getCount() == 0)
-- The streamer reports a viewer: a picture goes out with its BMP header, one at a time.
channel(info_name):push("listening=13346")
channel(info_name):push("clients=1")
frame(0.06)
local stream = channel(frames_name)
assert(stream:getCount() == 1, "one picture for the stream")
local job = stream.items[1]
assert(#job.header == 122 and #job.data.bytes == 640 * 360 * 4)
assert(job.data.released, "the game's own reference to a picture goes as soon as the sender has it")
assert(state().stream_status == "listening" and state().stream_clients == "1")
assert(state().stream_epoch:match("^%d+%.1$"), "a stream id that differs from one game session to the next")
channel(info_name):push("fps=29")
frame(0.06)
assert(state().stream_fps == "29")
frame(0.06)
assert(stream:getCount() == 1, "no new picture while the last one is still being sent")
stream:pop()
frame(0.06)
assert(stream:getCount() == 1, "the next picture once the sender is free")
stream:pop()

-- A snapshot after a move, when asked: once, into two files in turn.
stream:clear()
files["balatro_meta/control.txt"] = "n=2\nlock=1\nspeed=1\nstream=1\nwidth=640\nfps=30\nsnap=7\n"
frame()
frame()
assert(files["balatro_meta/snap_1.png"] == "PNG:" .. 640 * 360 * 4)
assert(state().snap == "7" and state().snap_file == "snap_1.png")
files["balatro_meta/snap_1.png"] = nil
frame()
assert(files["balatro_meta/snap_1.png"] == nil, "the same snapshot is not taken twice")
files["balatro_meta/control.txt"] = "n=3\nlock=1\nspeed=1\nstream=1\nwidth=640\nfps=30\nsnap=1234567890\n"
frame()
frame()
assert(files["balatro_meta/snap_0.png"] and state().snap == "1234567890" and state().snap_file == "snap_0.png")

-- Busy while blocking events are queued; non-blocking ones don't count.
G.E_MANAGER.queues.base = { { blocking = true }, { blocking = false }, {} }
frame()
assert(state().busy == "2")
G.E_MANAGER.queues.base = {}

-- Pictures as files when the viewer asks (no stream): rotating over three files.
files["balatro_meta/control.txt"] = "n=4\nlock=1\nspeed=2\nframes=1\nevery=0.2\nwidth=640\n"
local seen = {}
for _ = 1, 6 do frame(0.25); if state().frame then seen[state().frame] = true end end
local n = 0
for _ in pairs(seen) do n = n + 1 end
assert(n == 3, "frames rotate over three files")
assert(stream.items[#stream.items] == "stop", "the stream stops when not wanted")
assert(G.SETTINGS.GAMESPEED == 2)

-- The viewer goes away: input, speed and frame cap come back.
for _ = 1, 30 do frame(0.3) end
love.mousepressed()
assert(calls.pressed == 2, "input must come back once the viewer is gone")
assert(G.SETTINGS.GAMESPEED == 4 and G.FPS_CAP == 60)
assert(state().locked == "0")

-- Wanted again: a new start with its own channels, so the old one's "stop" can't reach it.
local epoch = state().stream_epoch
files["balatro_meta/control.txt"] = "n=6\nlock=1\nspeed=1\nstream=1\n"
frame()
local st2 = threads[#threads]
assert(st2.code == M.STREAMER and st2.started[2] ~= frames_name, "new channels for a new start")
assert(state().stream_epoch ~= epoch and state().stream_status == "starting")
assert(channel(st2.started[2]):getCount() == 0)

-- In the menu with no run-setup deck (after a run ends): the viewer's mod makes the default one.
G.STATE = G.STATES.MENU
frame()
assert(G.GAME.viewed_back and G.GAME.viewed_back.name == "Red Deck")
local made = G.GAME.viewed_back
frame()
assert(G.GAME.viewed_back == made, "an existing one is left alone")
G.STATE = 5

-- Max speed uncaps the frame rate.
files["balatro_meta/control.txt"] = "n=7\nlock=1\nspeed=10\nuncap=1\n"
frame()
assert(G.SETTINGS.GAMESPEED == 10 and G.FPS_CAP == nil)
assert(not M.error, tostring(M.error))
assert(released > 0)
print("watch mod: all checks passed")
