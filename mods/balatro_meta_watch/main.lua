-- Balatro Meta Watch: the companion of `make watch` (Balatro Meta project).
--
-- While the viewer runs, this mod:
--   * blocks mouse and keyboard, so nothing disturbs the bot's moves (a small badge says so);
--   * sets the game speed the viewer asks for;
--   * streams the game to the viewer page as live video (about 30 frames a second) from a tiny web
--     server on this computer only (127.0.0.1, port = BalatroBot's port + 1000); nothing is written
--     to disk for it;
--   * saves one picture after each move when the viewer asks (for clicking back through a run);
--   * reports whether the game is still playing out the last move (queued animations), so the
--     viewer waits before sending the next one.
--   * in the menu, makes sure BalatroBot can start the next run (see ensure_viewed_back).
-- It changes nothing in the rules. It talks to the viewer through files next to the game's saves
-- (love.filesystem): balatro_meta/control.txt in; balatro_meta/state.txt and snap_0/1.png out. If
-- the viewer stops (no fresh control file for a few seconds), input comes back, the speed goes back
-- to what it was, and the stream stops.

local VERSION = "0.3.0"
local DIR = "balatro_meta"
local CONTROL = DIR .. "/control.txt"
local STATE = DIR .. "/state.txt"
local STALE = 6 -- seconds without a new control stamp before the viewer counts as gone
local PORT = (tonumber(os.getenv("BALATROBOT_PORT")) or 12346) + 1000

local M = {
    version = VERSION,
    port = PORT,
    boot = os.time(), -- tells this game session's stream apart from the last one's
    control = {},
    stamp = nil,
    fresh_at = -1e9,
    last_read = -1e9,
    last_state = -1e9,
    last_frame = -1e9,
    last_stream = -1e9,
    seq = 0,
    slot = 0,
    snap_slot = 0,
    encoding = 0, -- jobs the encoder still has
    canvases = {},
    saved = nil,
    frame = nil,
    snap_taken = nil,
    snap_done = nil,
    snap_file = nil,
    stream_status = "off",
    stream_clients = 0,
    stream_fps = 0,
    stream_count = 0,
    stream_epoch = "",
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

-- BalatroBot's `start` sets the deck on the run-setup screen's deck (G.GAME.viewed_back). Steamodded's
-- run-select screen doesn't make that deck when there is no saved run to continue, which is always the
-- case once a run has ended, so every `start` after a lost run would fail. While the viewer runs, make
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

-- ------------------------------------------------------------------ pictures
-- A BMP header for raw RGBA rows, top to bottom (V4 header with channel masks), so the stream sends
-- the canvas bytes as they are. Browsers show a multipart stream of these in an <img>.
local function le16(n)
    return string.char(n % 256, math.floor(n / 256) % 256)
end
local function le32(n)
    n = n % 4294967296
    return string.char(n % 256, math.floor(n / 256) % 256, math.floor(n / 65536) % 256, math.floor(n / 16777216) % 256)
end
function M.bmp_header(w, h)
    local size = w * h * 4
    return "BM" .. le32(122 + size) .. le16(0) .. le16(0) .. le32(122)
        .. le32(108) .. le32(w) .. le32(-h) .. le16(1) .. le16(32) .. le32(3) .. le32(size)
        .. le32(2835) .. le32(2835) .. le32(0) .. le32(0)
        .. le32(0x000000FF) .. le32(0x0000FF00) .. le32(0x00FF0000) .. le32(0xFF000000)
        .. "BGRs" .. string.rep("\0", 36) .. le32(0) .. le32(0) .. le32(0)
end

-- PNG encoding and writing (snapshots, and pictures when the stream can't run) on a LOVE thread.
M.ENCODER = [[
require("love.image")
require("love.filesystem")
local inbox = love.thread.getChannel("bmwatch_encode")
local outbox = love.thread.getChannel("bmwatch_encoded")
while true do
    local job = inbox:demand()
    if job == "stop" then
        return
    end
    local ok, err = pcall(function()
        job.data:encode("png", job.path)
    end)
    if job.data.release then
        job.data:release() -- the picture's memory goes now, not at the next garbage collection
    end
    outbox:push({ kind = job.kind, name = job.name, n = job.n, w = job.w, h = job.h, ok = ok,
                  err = err and tostring(err) or nil })
end
]]

-- The live stream: a tiny web server on 127.0.0.1 that sends each new picture to every viewer as a
-- part of a multipart/x-mixed-replace response. Each start of it has its own channels, so a stopping
-- one never takes pictures or messages meant for the next.
M.STREAMER = [[
local port, frames_name, info_name = ...
require("love.image")
local socket = require("socket")
local frames = love.thread.getChannel(frames_name)
local info = love.thread.getChannel(info_name)
local server, err
for _ = 1, 10 do -- (the last server may still be closing)
    server, err = socket.bind("127.0.0.1", port)
    if server then
        break
    end
    socket.sleep(0.3)
end
if not server then
    info:push("error=" .. tostring(err))
    return
end
server:settimeout(0)
info:push("listening=" .. port)
local clients = {}
local sent, since = 0, socket.gettime()
while true do
    local c = server:accept()
    while c do
        c:settimeout(1)
        local line = c:receive("*l") or ""
        while true do
            local h = c:receive("*l")
            if not h or h == "" then
                break
            end
        end
        if line:match("^GET /stream") then
            c:settimeout(2)
            c:setoption("tcp-nodelay", true) -- each picture goes out whole, at once
            c:send("HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=bmframe\r\n"
                .. "Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n")
            clients[#clients + 1] = c
            info:push("clients=" .. #clients)
        else
            c:send("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            c:close()
        end
        c = server:accept()
    end
    local job = frames:demand(0.2)
    if job == "stop" then
        for _, cl in ipairs(clients) do
            cl:close()
        end
        server:close()
        info:push("stopped=1")
        return
    end
    if job and #clients > 0 then
        local body = job.header .. job.data:getString()
        local part = "--bmframe\r\nContent-Type: image/bmp\r\nContent-Length: " .. #body .. "\r\n\r\n" .. body .. "\r\n"
        for i = #clients, 1, -1 do
            local ok = clients[i]:send(part)
            if not ok then
                clients[i]:close()
                table.remove(clients, i)
                info:push("clients=" .. #clients)
            end
        end
        sent = sent + 1
    end
    if job and job.data and job.data.release then
        job.data:release() -- the picture's memory goes now, not at the next garbage collection
    end
    local t = socket.gettime()
    if t - since >= 1 then
        info:push("fps=" .. math.floor(sent / (t - since) + 0.5))
        sent, since = 0, t
    end
end
]]

local function encoder()
    if M.encoder == nil then
        M.encoder = love.thread.newThread(M.ENCODER)
        M.encoder:start()
        M.encode_in = love.thread.getChannel("bmwatch_encode")
        M.encode_out = love.thread.getChannel("bmwatch_encoded")
    end
end

local function collect_encoded()
    if not M.encode_out then
        return
    end
    while true do
        local done = M.encode_out:pop()
        if not done then
            break
        end
        M.encoding = math.max(0, M.encoding - 1)
        if not done.ok then
            M.error = done.err
        elseif done.kind == "snap" then
            M.snap_done = done.n
            M.snap_file = done.name
        else
            M.seq = M.seq + 1
            M.frame = { name = done.name, w = done.w, h = done.h }
        end
    end
    if M.encoder and M.encoder:getError() then
        M.error = M.encoder:getError()
    end
end

local function start_stream()
    if M.streamer then
        return
    end
    M.stream_count = M.stream_count + 1
    M.stream_epoch = M.boot .. "." .. M.stream_count
    local frames_name = "bmwatch_stream_" .. M.stream_epoch
    local info_name = "bmwatch_stream_info_" .. M.stream_epoch
    M.stream_frames = love.thread.getChannel(frames_name)
    M.stream_info = love.thread.getChannel(info_name)
    M.streamer = love.thread.newThread(M.STREAMER)
    M.streamer:start(PORT, frames_name, info_name)
    M.stream_status = "starting"
    M.stream_clients = 0
    M.stream_fps = 0
end

local function stop_stream()
    if M.streamer then
        M.stream_frames:push("stop")
        M.streamer = nil
    end
    M.stream_status = "off"
    M.stream_clients = 0
    M.stream_fps = 0
end

local function poll_stream()
    if not M.stream_info then
        return
    end
    while true do
        local msg = M.stream_info:pop()
        if not msg then
            break
        end
        local k, v = tostring(msg):match("^(%w+)=(.*)$")
        if k == "clients" then
            M.stream_clients = tonumber(v) or 0
        elseif k == "fps" then
            M.stream_fps = tonumber(v) or 0
        elseif k == "listening" then
            M.stream_status = "listening"
        elseif k == "error" then
            M.stream_status = "error"
            M.stream_failed_at = now()
            M.error = "stream: " .. v
            M.streamer = nil
        end
    end
    if M.streamer and M.streamer:getError() then
        M.stream_status = "error"
        M.stream_failed_at = now()
        M.error = "stream: " .. tostring(M.streamer:getError())
        M.streamer = nil
    end
end

-- A copy of the game's own canvas (before screen effects) at `width` pixels wide, as ImageData.
local function grab(width)
    local src = G and G.CANVAS
    if not src then
        return nil
    end
    local sw, sh = src:getDimensions()
    local w = math.max(64, math.min(width, sw))
    local h = math.max(36, math.floor(sh * w / sw + 0.5))
    local key = w .. "x" .. h -- one canvas per size: the stream's and the snapshots' (plain 8-bit RGBA)
    local small = M.canvases[key]
    if not small then
        small = love.graphics.newCanvas(w, h, { format = "rgba8" })
        M.canvases[key] = small
    end
    local fmin, fmag = src:getFilter()
    love.graphics.push("all")
    local ok, err = pcall(function()
        love.graphics.setCanvas(small)
        love.graphics.origin()
        love.graphics.setShader()
        love.graphics.setBlendMode("alpha", "alphamultiply")
        love.graphics.clear(0, 0, 0, 1)
        love.graphics.setColor(1, 1, 1, 1)
        src:setFilter("linear", "linear")
        love.graphics.draw(src, 0, 0, 0, w / sw, h / sh)
    end)
    src:setFilter(fmin, fmag)
    love.graphics.pop() -- always: the game's own drawing state comes back even if the copy failed
    if not ok then
        error(err)
    end
    return small:newImageData(), w, h
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
        "stream_port=" .. PORT,
        "stream_status=" .. M.stream_status,
        "stream_clients=" .. M.stream_clients,
        "stream_fps=" .. M.stream_fps,
        "stream_epoch=" .. M.stream_epoch,
    }
    if M.snap_done then
        lines[#lines + 1] = "snap=" .. string.format("%d", M.snap_done)
        lines[#lines + 1] = "snap_file=" .. tostring(M.snap_file)
    end
    if M.frame then
        lines[#lines + 1] = "frame=" .. M.frame.name
        lines[#lines + 1] = "width=" .. M.frame.w
        lines[#lines + 1] = "height=" .. M.frame.h
    end
    if M.error then
        lines[#lines + 1] = "error=" .. tostring(M.error):gsub("[\r\n]", " ")
    end
    lines[#lines + 1] = "end=1" -- the viewer ignores a report it reads half-written
    love.filesystem.write(STATE, table.concat(lines, "\n") .. "\n")
end

-- Every frame, in love.update (which runs even in BalatroBot's headless mode).
function M.update()
    local t = now()
    collect_encoded()
    poll_stream()
    if active() then
        ensure_viewed_back()
    end
    if t - M.last_read >= 0.1 then
        M.last_read = t
        read_control()
        apply_speed()
        if active() and M.control.stream == "1" then
            -- (After an error, such as the port being taken, try again every 10 seconds.)
            if not M.streamer and (M.stream_status ~= "error" or t - (M.stream_failed_at or 0) > 10) then
                start_stream()
            end
        elseif M.streamer then
            stop_stream()
        end
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

-- Hand a picture to a thread; the thread's copy is the only one kept (released here at once).
local function hand_over(channel, job)
    channel:push(job)
    if job.data.release then
        job.data:release()
    end
end

-- After the game has drawn a frame: pictures for the stream, a snapshot, or (no stream) a file.
function M.draw()
    local t = now()
    if not active() then
        return
    end
    local width = tonumber(M.control.width) or 1280
    -- The live stream: only while someone watches, and never ahead of the sender.
    local fps = tonumber(M.control.fps) or 30
    if M.streamer and M.stream_clients > 0 and t - M.last_stream >= 1 / fps
        and M.stream_frames:getCount() == 0 then
        M.last_stream = t
        local data, w, h = grab(width)
        if data then
            hand_over(M.stream_frames, { data = data, header = M.bmp_header(w, h) })
        end
    end
    -- One picture after a move, when asked (two files in turn, so the viewer never reads one that is
    -- being rewritten).
    local ask = tonumber(M.control.snap)
    if ask and ask ~= M.snap_taken then
        M.snap_taken = ask
        local data, w, h = grab(math.min(width, 960))
        if data then
            encoder()
            M.encoding = M.encoding + 1
            M.snap_slot = 1 - M.snap_slot
            local name = "snap_" .. M.snap_slot .. ".png"
            hand_over(M.encode_in, { kind = "snap", data = data, path = DIR .. "/" .. name, name = name, n = ask,
                                     w = w, h = h })
        end
    end
    -- Pictures as files, a few a second, only if the viewer asks (when the stream can't run).
    if M.control.frames == "1" and M.encoding == 0 and t - M.last_frame >= (tonumber(M.control.every) or 0.25) then
        M.last_frame = t
        local data, w, h = grab(math.min(width, 960))
        if data then
            M.slot = (M.slot + 1) % 3
            local name = "frame_" .. M.slot .. ".png"
            encoder()
            M.encoding = M.encoding + 1
            hand_over(M.encode_in, { kind = "frame", data = data, path = DIR .. "/" .. name, name = name, w = w,
                                     h = h })
        end
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
