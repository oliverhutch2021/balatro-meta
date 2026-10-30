-- Runs the mod's stream server (M.STREAMER) under LuaJIT + LuaSocket with stand-in LOVE channels that
-- produce a moving test picture 30 times a second: `luajit tests/lua/run_streamer.lua <port> [seconds]`.
-- tests/python/test_watch_mod.py connects to it as a browser would.
local socket = require("socket")
local port, seconds = tonumber(arg[1]) or 13399, tonumber(arg[2]) or 20
local W, H = 64, 36
local files = {}
local function stub()
    local t = {}
    setmetatable(t, { __index = function() return function() end end })
    return t
end
love = {
    timer = { getTime = function() return socket.gettime() end },
    filesystem = { read = function() return nil end, write = function() return true end,
                   createDirectory = function() return true end },
    graphics = stub(), mouse = stub(), thread = stub(),
}
G = nil
dofile("mods/balatro_meta_watch/main.lua")
local M = BALATRO_META_WATCH
local start, n = socket.gettime(), 0
local channels = {}
local function channel(name)
    if channels[name] then return channels[name] end
    local c = {}
    if name == "frames" then
        function c:demand(timeout)
            if socket.gettime() - start > seconds then return "stop" end
            socket.sleep(1 / 30)
            n = n + 1
            local px = {}
            for y = 0, H - 1 do
                for x = 0, W - 1 do
                    px[#px + 1] = string.char((x * 4 + n) % 256, y * 7 % 256, 90, 255)
                end
            end
            local bytes = table.concat(px)
            return { header = M.bmp_header(W, H), data = { getString = function() return bytes end } }
        end
    else
        function c:push(v) io.stdout:write(tostring(v), "\n"); io.stdout:flush() end
    end
    channels[name] = c
    return c
end
package.preload["love.image"] = function() end
love.thread = { getChannel = channel }
assert(loadstring(M.STREAMER))(port, "frames", "info")
