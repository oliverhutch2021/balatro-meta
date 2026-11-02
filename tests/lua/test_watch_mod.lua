-- The companion mod against a stand-in for LOVE and the game: run with `luajit tests/lua/test_watch_mod.lua`.
local files = {}
local clock = 100.0
local calls = { pressed = 0, drawn = 0, updated = 0 }
local dirs = {}

love = {
    timer = { getTime = function() return clock end },
    filesystem = {
        read = function(name) return files[name] end,
        write = function(name, d) files[name] = d; return true end,
        createDirectory = function(name) dirs[#dirs + 1] = name; return true end,
    },
    graphics = {
        push = function() end, pop = function() end, setCanvas = function() end, origin = function() end,
        setShader = function() end, setColor = function() end, rectangle = function() end, print = function() end,
        getWidth = function() return 1920 end,
        getFont = function() return { getWidth = function() return 100 end, getHeight = function() return 12 end } end,
    },
    mouse = { getPosition = function() return 500, 400 end, isDown = function() return true end },
    audio = { volume = 0.7, getVolume = function() return love.audio.volume end,
              setVolume = function(v) love.audio.volume = v end },
    mousepressed = function() calls.pressed = calls.pressed + 1 end,
    update = function() calls.updated = calls.updated + 1 end,
    draw = function() calls.drawn = calls.drawn + 1 end,
}
G = {
    SETTINGS = { GAMESPEED = 4 }, FPS_CAP = 60,
    STATES = { SHOP = 5, ROUND_EVAL = 8, MENU = 11 }, STATE = 5, STATE_COMPLETE = true,
    E_MANAGER = { queues = { base = {} } },
    GAME = {}, P_CENTERS = { b_red = { name = "Red Deck" } },
}
Back = setmetatable({}, { __call = function(_, center) return { name = center.name } end })

-- The checks' game runs on BalatroBot's port 12350: the mod's files live in a folder for that port.
local getenv = os.getenv
os.getenv = function(k) if k == "BALATROBOT_PORT" then return "12350" end return getenv(k) end
dofile("mods/balatro_meta_watch/main.lua")
os.getenv = getenv
local M = BALATRO_META_WATCH
assert(M.port == 12350 and M.dir == "balatro_meta/12350" and dirs[1] == "balatro_meta/12350")
local CONTROL, STATE = "balatro_meta/12350/control.txt", "balatro_meta/12350/state.txt"

local function frame(dt) clock = clock + (dt or 0.3); love.update(1 / 60); love.draw() end
local function state()
    local t = {}
    for k, v in (files[STATE] or ""):gmatch("([%w_]+)=([^\n]*)") do t[k] = v end
    return t
end

-- No checks yet: input passes, the speed is the game's own, the menu is untouched, the state is reported.
G.STATE = G.STATES.MENU
frame()
love.mousepressed()
assert(calls.pressed == 1 and calls.drawn == 1 and calls.updated == 1)
assert(select(1, love.mouse.getPosition()) == 500)
local s = state()
assert(s.version == "0.6.0" and s.port == "12350" and s.locked == "0" and s["end"] == "1")
assert(G.SETTINGS.GAMESPEED == 4 and G.GAME.viewed_back == nil)
G.STATE = 5

-- The checks start: no lock, no speed asked: nothing about the game changes.
files[CONTROL] = "n=1\nlock=0\n"
frame()
assert(G.SETTINGS.GAMESPEED == 4 and G.FPS_CAP == 60 and state().locked == "0" and state().control == "1")
love.mousepressed()
assert(calls.pressed == 2)

-- In the menu with no run-setup deck (after a run ends): the mod makes the default one, once.
G.STATE = G.STATES.MENU
frame()
assert(G.GAME.viewed_back and G.GAME.viewed_back.name == "Red Deck")
local made = G.GAME.viewed_back
frame()
assert(G.GAME.viewed_back == made, "an existing one is left alone")
G.STATE = 5

-- Busy while blocking events are queued; non-blocking or finished ones don't count.
G.E_MANAGER.queues.base = { { blocking = true }, { blocking = false }, { complete = true }, {} }
files[CONTROL] = "n=2\nlock=0\n"
frame(0.06)
assert(state().busy == "2")
G.E_MANAGER.queues.base = {}
frame(0.06)
assert(state().busy == "0")

-- Reports come often while the checks run (every 0.05 s), once a second otherwise.
local before = state().time
frame(0.06)
assert(state().time ~= before, "a fresh report")

-- The checks' sound: untouched unless asked; muted while asked.
assert(love.audio.volume == 0.7 and state().muted == "0")
files[CONTROL] = "n=21\nlock=0\nmute=1\n"
frame()
assert(love.audio.volume == 0 and state().muted == "1")
love.audio.volume = 0.5 -- the game sets it itself: muted again by the next read
files[CONTROL] = "n=22\nlock=0\nmute=1\n"
frame()
assert(love.audio.volume == 0, "set again while asked")

-- Asked to: lock, speed and an uncapped frame rate (and still muted).
files[CONTROL] = "n=3\nlock=1\nspeed=10\nuncap=1\nmute=1\n"
frame()
love.mousepressed()
assert(calls.pressed == 2, "a click while locked must be swallowed")
assert(select(1, love.mouse.getPosition()) == -200 and love.mouse.isDown(1) == false)
assert(G.SETTINGS.GAMESPEED == 10 and G.FPS_CAP == nil and state().locked == "1")

-- The checks go away: input, speed and frame cap come back.
for _ = 1, 30 do frame(0.3) end
love.mousepressed()
assert(calls.pressed == 3, "input must come back once the checks are gone")
assert(G.SETTINGS.GAMESPEED == 4 and G.FPS_CAP == 60)
assert(state().locked == "0")
assert(love.audio.volume == 0.7 and state().muted == "0", "the sound comes back as it was")

-- How a run ends is noted and kept until another run ends, so a recorder that was waiting for the game to settle
-- still learns it when the player moves on fast: lost (the game-over screen), then won (Endless after it changes
-- nothing). Reports come once a second now that the checks are gone.
G.STATES.GAME_OVER = 4
G.GAME = { pseudorandom = { seed = "SEEDLOST" }, round_resets = { ante = 2 }, round = 5 }
frame(1.1)
s = state()
assert(s.seed == "SEEDLOST" and s.won == "0" and s.ante == "2" and s.round == "5" and s.last_end == "")
G.STATE = G.STATES.GAME_OVER
frame(1.1)
assert(state().last_end == "SEEDLOST:lost:2:5:GAME_OVER", state().last_end)
G.GAME = { pseudorandom = { seed = "SEEDWON1" }, round_resets = { ante = 1 }, round = 1 }
G.STATE = 5
frame(1.1)
assert(state().seed == "SEEDWON1" and state().last_end == "SEEDLOST:lost:2:5:GAME_OVER", "kept while the next run plays")
G.GAME.won, G.GAME.round_resets.ante, G.GAME.round, G.STATE = true, 8, 24, G.STATES.ROUND_EVAL
frame(1.1)
assert(state().won == "1" and state().last_end == "SEEDWON1:won:8:24:ROUND_EVAL", state().last_end)
G.STATE = G.STATES.GAME_OVER
frame(1.1)
assert(state().last_end == "SEEDWON1:won:8:24:ROUND_EVAL", "a win stays a win")
G.GAME = {}
frame(1.1)
assert(state().seed == "" and state().won == "0" and state().last_end == "SEEDWON1:won:8:24:ROUND_EVAL")
G.STATE = 5

-- A Balatro on the default port uses its own folder (never the checks' files).
for k in pairs(package.loaded) do if k:match("main") then package.loaded[k] = nil end end
dofile("mods/balatro_meta_watch/main.lua")
assert(BALATRO_META_WATCH.port == 12346 and BALATRO_META_WATCH.dir == "balatro_meta/12346")
assert(not M.error, tostring(M.error))
print("watch mod: all checks passed")
